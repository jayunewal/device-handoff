// SPDX-License-Identifier: Apache-2.0

// The Mac's side of sharing a window, or a whole screen: the picker of the system lets the person
// choose which, ScreenCaptureKit captures it, and VideoToolbox encodes what it captures
// (BatonH264Encoder.swift).
// What decides when a picture is made, and what becomes of it, is in BatonSource.swift, which does
// not depend on any of this.
//
// The system's picker is used, and not a list of windows made by the app, for two reasons: the
// person chooses at the place where the system shows what a window or a screen is, and an app that
// is given one by the picker needs no permission to record the screen, which an app that builds for
// a free Apple ID would lose again with every build.
//
// The one case with no picker is a paired device that the person allowed beforehand to look at the
// main screen of this Mac and asked for it (ADR 0010): nobody is here to choose, so the screen is
// taken without the picker, and that is where the permission to record the screen is needed.
//
// A window or a whole screen can be steered by the person at the other device too
// (BatonInput.swift, and BatonControl.swift for the calls of the system). A screen that a paired
// device steers, after it started the session, is private (BatonPrivacy.swift and
// BatonPrivacyMac.swift): it is dark for the people at this Mac, and they cannot use its keys or its
// pointer, until the session is over and the Mac is locked again.
//
// An iPhone or an iPad cannot share a window, so there is nothing here for them.

#if os(macOS)

import CoreGraphics
import CoreMedia
import CoreVideo
import Foundation
import ScreenCaptureKit
import os

/// The log of the system, for the sizes that decide whether a touch can be placed in a picture.
/// They are public so that `log show` has them whole: a line made with `NSLog` and a string
/// argument may be shown as `<private>`.
private let layoutLog = Logger(subsystem: "io.github.jayunewal.baton", category: "layout")

/// What was picked, as far as the system says: the number of a window and the process that owns
/// it, or the number of a display. The system tells them from macOS 15.2 on.
private enum PickedTarget {
  case window(UInt32, process: Int32)
  case display(UInt32)
}

/// What the system answers when it is asked which screens there are, handed from the thread it
/// answers on to the one that waits.
@available(macOS 12.3, *)
private final class ShareableContentAnswer: @unchecked Sendable {
  private let lock = NSLock()
  private let arrived = DispatchSemaphore(value: 0)
  private var content: SCShareableContent?
  private var failure: (any Error)?

  func give(_ content: SCShareableContent?, _ failure: (any Error)?) {
    lock.lock()
    self.content = content
    self.failure = failure
    lock.unlock()
    arrived.signal()
  }

  /// nil if the system did not answer within `seconds`. An answer that comes later is dropped.
  func wait(seconds: Double) -> (content: SCShareableContent?, failure: (any Error)?)? {
    guard arrived.wait(timeout: .now() + seconds) == .success else { return nil }
    lock.lock()
    defer { lock.unlock() }
    return (content, failure)
  }
}

/// How long the system is waited for when it is asked which screens there are.
private let screenListingSeconds = 5.0

/// The picker of the system, as the sharer of this Mac. There is one for the life of the process.
@available(macOS 14.0, *)
final class MacSharer: NSObject, SharerPlatform, SCContentSharingPickerObserver {
  /// A window or a screen can be steered where the system says which the person picked, and where
  /// the person has let this app post input events. It is asked again each time, since the person
  /// may let the app in System Settings at any time.
  var canControl: Bool { macControlAccess() == .allowed }

  /// The main screen can be given with no picker, if the person has let this app record the screen
  /// (`screen` says when they have not).
  var canGiveScreen: Bool { true }

  /// The sound of a whole screen can be sent where the system has taps of the sound of the Mac,
  /// which is from macOS 14.2. Whether the person has let the app record it is found out when
  /// someone listens (`SoundPump`).
  var canSendSound: Bool {
    if #available(macOS 14.2, *) { return true }
    return false
  }

  private let lock = NSLock()
  /// The request that the picker is up for.
  private var request: UInt64?
  /// Whether this is an observer of the picker already. Only the main thread touches it.
  private var observing = false
  /// Whether the system was asked, in this run of the app, to let it record the screen.
  private var askedToRecordTheScreen = false
  /// Whether the system was asked, in this run of the app, to let it hold the keys and the pointer.
  private var askedToHoldTheInput = false

  func pick(request: UInt64) throws {
    lock.lock()
    let busy = self.request != nil
    if !busy { self.request = request }
    lock.unlock()
    if busy {
      throw SharerFailure(sentence: "A window or screen is being chosen already.")
    }
    // The picker is an AppKit window, and AppKit is used from the main thread. The answer comes
    // through the observer, later.
    DispatchQueue.main.async { [self] in
      let picker = SCContentSharingPicker.shared
      if !observing {
        picker.add(self)
        observing = true
      }
      var configuration = SCContentSharingPickerConfiguration()
      // One window, or one whole display: the picker offers both, and the person chooses there. It
      // is presented with no style, because `present(using:)` is for a single type of selection
      // and would allow nothing else.
      configuration.allowedPickerModes = [.singleWindow, .singleDisplay]
      configuration.allowsChangingSelectedContent = false
      picker.defaultConfiguration = configuration
      picker.isActive = true
      picker.present()
    }
  }

  func cancelPick(request: UInt64) {
    lock.lock()
    let mine = self.request == request
    if mine { self.request = nil }
    lock.unlock()
    if mine {
      // What the person picks at a picker that is still up is not wanted: nobody asked for it.
      DispatchQueue.main.async { SCContentSharingPicker.shared.isActive = false }
    }
  }

  // MARK: - The main screen, with no picker

  func screen(privacy: ScreenPrivacy) throws -> (window: SharedWindow, info: PickedWindowInfo) {
    // A window or a screen that the person picked is captured with no permission to record the
    // screen. One that nobody picked is not: the system wants that permission, and the person at
    // this Mac gives it in System Settings. The call does not wait for them, and the permission
    // takes effect when the app is opened again.
    guard CGPreflightScreenCaptureAccess() else {
      askToRecordTheScreen()
      throw ScreenRefusal.needsPermission
    }
    // A private screen is given only to an app that can keep it private. The cover goes up when the
    // session starts, and not now, so that a screen that is given and never used changes nothing.
    if privacy == .private, !macCanHoldTheInput() {
      askToHoldTheInput()
      throw ScreenRefusal.needsInputPermission
    }
    let display = try mainDisplay()
    let filter = SCContentFilter(display: display, excludingWindows: [])
    guard let size = MacSharer.pixelSize(of: filter) else {
      throw SharerFailure(sentence: "The screen has no size to share.")
    }
    // A screen that is given this way is not named: Rust reads no title and no application.
    let info = PickedWindowInfo(
      width: size.width, height: size.height, title: "", app: "", display: true)
    let window = MacSharedWindow(
      filter: filter, info: info, target: .display(display.displayID), privacy: privacy)
    return (window, info)
  }

  /// Asks the system to show its question about recording the screen. It shows its own question
  /// once, and does not wait for the person, so it is asked once in each run of the app, and not
  /// from the thread that waits for `screen`.
  private func askToRecordTheScreen() {
    lock.lock()
    let first = !askedToRecordTheScreen
    askedToRecordTheScreen = true
    lock.unlock()
    guard first else { return }
    DispatchQueue.main.async { _ = CGRequestScreenCaptureAccess() }
  }

  /// Asks the system to show its questions about holding the keys and the pointer. As for the
  /// recording of the screen, they are shown once, they do not wait for the person, and they are
  /// asked once in each run of the app.
  private func askToHoldTheInput() {
    lock.lock()
    let first = !askedToHoldTheInput
    askedToHoldTheInput = true
    lock.unlock()
    guard first else { return }
    requestMacInputHoldAccess()
  }

  /// The display that has the menu bar. The system is waited for a few seconds at most.
  private func mainDisplay() throws -> SCDisplay {
    let answer = ShareableContentAnswer()
    SCShareableContent.getExcludingDesktopWindows(true, onScreenWindowsOnly: true) {
      content, failure in
      answer.give(content, failure)
    }
    guard let answered = answer.wait(seconds: screenListingSeconds) else {
      throw SharerFailure(sentence: "This Mac did not say which screens it has.")
    }
    if let failure = answered.failure {
      let error = failure as NSError
      // The person may have taken the permission back since it was looked at.
      if error.domain.hasSuffix("SCStreamErrorDomain"), error.code == CaptureErrorCode.userDeclined {
        throw ScreenRefusal.needsPermission
      }
      NSLog("Baton: the screens could not be listed (%@, %ld).", error.domain, error.code)
      throw SharerFailure(sentence: "This Mac did not say which screens it has.")
    }
    // A display that is not in the list is not on: the Mac is asleep, or has none connected.
    let main = CGMainDisplayID()
    guard let display = answered.content?.displays.first(where: { $0.displayID == main }) else {
      throw ScreenRefusal.unavailable
    }
    return display
  }

  // MARK: - The answers of the picker

  func contentSharingPicker(_ picker: SCContentSharingPicker, didCancelFor stream: SCStream?) {
    answer(.gaveUp)
  }

  func contentSharingPicker(
    _ picker: SCContentSharingPicker, didUpdateWith filter: SCContentFilter, for stream: SCStream?
  ) {
    // A stream that the picker was shown for is not one of ours: none is.
    if stream != nil { return }
    answer(MacSharer.choice(of: filter))
  }

  func contentSharingPickerStartDidFailWithError(_ error: any Error) {
    NSLog("Baton: the picker could not be shown (%ld).", (error as NSError).code)
    answer(.failed("The picker of the system could not be shown."))
  }

  /// Gives the answer to the request that waits for one, if one does.
  private func answer(_ answer: PickAnswer) {
    lock.lock()
    let waiting = request
    request = nil
    lock.unlock()
    // The picker is touched from the main thread only, as `pick` and `cancelPick` do: the observer
    // is called on a thread of the system's choosing.
    DispatchQueue.main.async { SCContentSharingPicker.shared.isActive = false }
    // Nobody asked, or nobody asks any more: a window that was chosen is let go of by dropping it.
    guard let waiting else { return }
    answerPick(request: waiting, answer)
  }

  /// What the person chose, as the answer to the request.
  private static func choice(of filter: SCContentFilter) -> PickAnswer {
    let display: Bool
    switch filter.style {
    case .window: display = false
    case .display: display = true
    default: return .failed("Choose a window or a screen.")
    }
    guard let size = pixelSize(of: filter) else {
      return .failed("The \(display ? "screen" : "window") has no size to share.")
    }
    var title = ""
    var app = ""
    var target: PickedTarget?
    // The system tells which window or display it is only from macOS 15.2 on. A display has neither
    // a title nor an application.
    if #available(macOS 15.2, *) {
      if display {
        if let shown = filter.includedDisplays.first {
          target = .display(shown.displayID)
        }
      } else if let window = filter.includedWindows.first {
        title = window.title ?? ""
        app = window.owningApplication?.applicationName ?? ""
        if let process = window.owningApplication?.processID {
          target = .window(window.windowID, process: process)
        }
      }
    }
    let info = PickedWindowInfo(
      width: size.width, height: size.height, title: title, app: app, display: display)
    return .chose(MacSharedWindow(filter: filter, info: info, target: target), info)
  }

  /// The size in pixels of what the filter lets through, or nil if it has none that a picture can
  /// have.
  private static func pixelSize(of filter: SCContentFilter) -> (width: UInt32, height: UInt32)? {
    let scale = Double(filter.pointPixelScale)
    let width = (Double(filter.contentRect.width) * scale).rounded()
    let height = (Double(filter.contentRect.height) * scale).rounded()
    guard width >= 1, height >= 1, width < 100_000, height < 100_000 else { return nil }
    return (UInt32(width), UInt32(height))
  }
}

/// A window, or a whole screen, that the person picked, shared from this Mac.
@available(macOS 14.0, *)
final class MacSharedWindow: SharedWindow {
  private let filter: SCContentFilter
  private let info: PickedWindowInfo
  /// Which window it is, if the system said.
  private let target: PickedTarget?
  private var format: PictureFormat?
  private var encoder: H264Encoder?
  private var capture: Capture?
  /// What the Rust side asks of a window that is steered. It is made by `start` on the thread that
  /// calls `surroundings` and `post`, one after the other, and that is the only one that uses it.
  private var control: WindowControl?
  /// Whether the people at this Mac may see the screen and use the Mac while it is shared.
  private let privacy: ScreenPrivacy
  /// The privacy of a session that is private, made by `start`.
  private var privateSession: PrivateSession?
  /// What sends the sound of a whole screen, made by `start`. A window has none.
  private var sound: SoundPump?

  fileprivate init(
    filter: SCContentFilter, info: PickedWindowInfo, target: PickedTarget?,
    privacy: ScreenPrivacy = .open
  ) {
    self.filter = filter
    self.info = info
    self.target = target
    self.privacy = privacy
  }

  func prepare(takes: ViewerTakes) throws -> PictureFormat {
    guard
      let chosen = chooseFormat(
        windowWidth: info.width, windowHeight: info.height, takes: takes)
    else {
      throw SharerFailure(sentence: "The other device cannot show a picture of this \(info.noun).")
    }
    do {
      encoder = try H264Encoder(format: chosen)
    } catch {
      NSLog("Baton: no video encoder could be made (%@).", String(describing: error))
      throw SharerFailure(sentence: "This device could not start the video encoder.")
    }
    format = chosen
    return chosen
  }

  func start(link: UInt64) throws {
    guard let format, let encoder, capture == nil else {
      throw SharerFailure(sentence: "This device could not start sharing the \(info.noun).")
    }
    let librarySession = LibrarySourceSession(link: link)
    // A private screen goes dark and out of reach of the people at this Mac now, before the first
    // picture and before the first event can arrive, and stays so until the session is over and
    // the Mac is locked. What is left of it when this fails is let go of by `released`.
    var ledger: PostLedger?
    if privacy == .private {
      let posts = PostLedger()
      let started = makePrivateSession(ledger: posts) { sentence in
        librarySession.end(sentence: sentence)
      }
      try started.begin()
      ledger = posts
      privateSession = started
    }
    // The sound of a whole screen goes with its pictures, when the person at the other device asks
    // for it (`hear`). A private screen keeps this Mac quiet from now until it is locked again,
    // whether or not the other device listens: nobody is here to hear it, and the people at home
    // would hear what the person at the other device does not. What `released` finds is let go of.
    if info.display, #available(macOS 14.2, *) {
      let pump = SoundPump(
        session: LibrarySoundSession(link: link), quiet: privacy == .private,
        makeTap: { MacSoundTap() }, makeEncoder: { try AacEncoder(form: $0) })
      sound = pump
      if privacy == .private { pump.keepQuiet() }
    }
    // The window server and the system are asked about the window, or the display, while it is
    // shared: where it is in each picture, and whether it can be seen. A display is never hidden.
    let listing = MacWindowListing()
    let activity = MacActivityReader()
    let note: (String) -> Void = { text in layoutLog.notice("Baton: \(text, privacy: .public)") }
    var layout: CaptureLayout?
    var window: UInt32?
    switch target {
    case .window(let id, _):
      layout = CaptureLayout(window: id, listing: listing, note: note)
      window = id
    case .display(let id):
      let displays = MacDisplayListing()
      layout = CaptureLayout(
        noun: "screen", bounds: { displays.bounds(ofDisplay: id) }, note: note)
    case nil:
      break
    }
    let made = Capture(
      session: librarySession, encoder: encoder, format: format, layout: layout,
      look: { [window] in watchedState(window: window, listing: listing, activity: activity) })
    switch target {
    case .window(let id, let process): control = makeWindowControl(window: id, process: process)
    case .display(let id): control = makeDisplayControl(display: id, ledger: ledger)
    case nil: break
    }
    do {
      // The system answers a start that fails (the person did not allow the recording, the window
      // went away) after this returns, and then the session is ended with a sentence.
      try made.begin(filter: filter)
    } catch {
      let failure = error as NSError
      NSLog("Baton: the capture could not be set up (%@, %ld).", failure.domain, failure.code)
      throw SharerFailure(sentence: "This device could not start capturing the \(info.noun).")
    }
    capture = made
  }

  func ended() {
    capture?.stop()
    encoder?.invalidate()
    // The Mac is locked before this returns, if it can be, and the screens and the keys come back
    // only then.
    privateSession?.finish(.sessionOver)
    // The Mac plays again: at once for a screen that is not private, and for a private one only now
    // that it is locked.
    sound?.close()
  }

  func released() {
    // The capture and the encoder were stopped by `ended`, if the window ever ran. A window that
    // was prepared and never started has an encoder to let go of.
    encoder?.invalidate()
    // A private screen that was raised and then did not start is locked and given back here. One
    // that was given and never started raised nothing, and changes nothing.
    privateSession?.finish(.released)
    sound?.close()
  }

  func surroundings() -> WindowSurroundings? {
    control?.surroundings()
  }

  func post(_ post: InputPost) -> Bool {
    control?.post(post) ?? false
  }

  func lift(screen: Bool, input: Bool) {
    // A screen that is not private keeps nothing from the people at this Mac, and has nothing to
    // lift.
    guard let privateSession else { return }
    privateSession.lift(screen: screen, input: input)
    // The people at this Mac hear it again while they can see what is on the screen, except while
    // the other device listens.
    sound?.screenShown(screen)
  }

  func hear(_ on: Bool) {
    sound?.hear(on)
  }
}

/// The capture of one window, and what is done with the pictures it makes.
///
/// Everything that touches the pump, and so the encoder, happens on `queue`: the images of the
/// capture arrive there, and so does the frame clock.
@available(macOS 14.0, *)
private final class Capture: NSObject, SCStreamOutput, SCStreamDelegate {
  private let queue = DispatchQueue(label: "io.github.jayunewal.baton.capture")
  private let session: SourceSession
  private let pump: FramePump<H264Encoder>
  private let format: PictureFormat
  /// How the window or the screen is laid out in each image, if it can be steered.
  private let layout: CaptureLayout?
  /// Pauses the session while the window cannot be seen, and resumes it when it can.
  private let pauses: PauseWatch

  /// Set by `begin` and `stop`, which the Rust side calls one after the other.
  private var stream: SCStream?
  private var clock: DispatchSourceTimer?
  private var watch: DispatchSourceTimer?

  /// `look` says whether the window can be seen, and is asked about once a second.
  init(
    session: SourceSession, encoder: H264Encoder, format: PictureFormat, layout: CaptureLayout?,
    look: @escaping () -> WatchedState?
  ) {
    self.session = session
    self.pump = FramePump(session: session, encoder: encoder)
    self.format = format
    self.layout = layout
    self.pauses = PauseWatch(session: session, look: look)
  }

  /// Throws if the capture cannot be set up. A capture that the system then fails to start ends
  /// the session, with a sentence.
  func begin(filter: SCContentFilter) throws {
    let configuration = SCStreamConfiguration()
    configuration.width = Int(format.width)
    configuration.height = Int(format.height)
    configuration.minimumFrameInterval = CMTime(value: 1, timescale: CMTimeScale(format.fps))
    // The planes the encoder takes as they are.
    configuration.pixelFormat = kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
    configuration.showsCursor = true
    configuration.capturesAudio = false
    // A window that is resized is scaled into the picture and keeps its shape.
    configuration.scalesToFit = true
    // The shadow of a window is not part of it. With the shadow the picture would be the window in
    // a black frame (a video has no transparency), scaled down to make room for it.
    configuration.ignoreShadowsSingleWindow = true
    configuration.queueDepth = 4

    let stream = SCStream(filter: filter, configuration: configuration, delegate: self)
    try stream.addStreamOutput(self, type: .screen, sampleHandlerQueue: queue)
    self.stream = stream

    // The frame clock: while the window does not change, nothing arrives from the capture, and the
    // pump is asked on this clock whether a picture is wanted all the same.
    let interval = DispatchTimeInterval.nanoseconds(Int(1_000_000_000 / max(format.fps, 1)))
    let timer = DispatchSource.makeTimerSource(queue: queue)
    timer.schedule(deadline: .now() + interval, repeating: interval, leeway: .milliseconds(2))
    timer.setEventHandler { [pump] in pump.tick() }
    clock = timer
    timer.resume()

    // Whether the window can still be seen, which is asked about on the same queue.
    let watcher = DispatchSource.makeTimerSource(queue: queue)
    watcher.schedule(deadline: .now() + 1, repeating: 1, leeway: .milliseconds(100))
    watcher.setEventHandler { [pauses] in pauses.poll() }
    watch = watcher
    watcher.resume()

    stream.startCapture { [weak self] error in
      if let error { self?.end(because: error) }
    }
  }

  /// Stops capturing, and waits for the work that is on its way to finish: the link of the session
  /// is closed already, so the calls into the library that it makes are answered at once.
  func stop() {
    clock?.cancel()
    clock = nil
    watch?.cancel()
    watch = nil
    if let stream {
      try? stream.removeStreamOutput(self, type: .screen)
      stream.stopCapture { _ in }
    }
    stream = nil
    queue.sync { pump.stop() }
  }

  /// The capture did not begin, or stopped: the session ends, with a sentence for the person if
  /// they did not stop it themselves. It is done from the queue, and so never from inside a call
  /// that the Rust side made, which the library would refuse.
  private func end(because error: any Error) {
    let error = error as NSError
    NSLog("Baton: the capture stopped (%@, %ld).", error.domain, error.code)
    queue.async { [pump, session] in
      pump.stop()
      session.end(sentence: captureStoppedSentence(domain: error.domain, code: error.code))
    }
  }

  // MARK: - What the system gives

  func stream(
    _ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer,
    of type: SCStreamOutputType
  ) {
    guard type == .screen, sampleBuffer.isValid else { return }
    // A frame of a window that did not change has no picture, and the status says so.
    guard
      let attachments = CMSampleBufferGetSampleAttachmentsArray(
        sampleBuffer, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
      let info = attachments.first,
      let raw = info[.status] as? Int, SCFrameStatus(rawValue: raw) == .complete,
      let image = CMSampleBufferGetImageBuffer(sampleBuffer)
    else { return }
    pump.captured(image, geometry: geometry(of: image, info))
  }

  /// How the window or the screen is laid out in this image, if it can be steered.
  private func geometry(of image: CVPixelBuffer, _ info: [SCStreamFrameInfo: Any])
    -> PictureGeometry?
  {
    guard let layout else { return nil }
    return layout.geometry(
      pictureWidth: UInt32(clamping: CVPixelBufferGetWidth(image)),
      pictureHeight: UInt32(clamping: CVPixelBufferGetHeight(image)),
      reported: screenRect(fromDictionary: info[.contentRect]))
  }

  func stream(_ stream: SCStream, didStopWithError error: any Error) {
    end(because: error)
  }
}

#endif
