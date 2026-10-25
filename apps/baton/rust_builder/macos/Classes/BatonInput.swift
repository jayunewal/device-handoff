// SPDX-License-Identifier: Apache-2.0

// What a Mac needs, apart from the calls of the system, to let the person at another device steer
// a window that it shares, or a whole display: the surroundings of the window or the display, which
// the Rust side asks for before it puts an event to it; the translation of what the person did into
// events of the Mac; the geometry of a picture; and the watch that pauses a session while the window
// cannot be seen.
// crates/baton-stream/src/gate.rs has the rules that the Rust side applies to what is answered
// here, and docs/adr/0009-window-streaming.md says why they are what they are.
//
// The calls of the system are given to it as protocols, and BatonControl.swift has the ones of a
// Mac. So that this can be built and tested on any computer that has a Swift compiler
// (apps/baton/rust_builder/swift_tests), nothing here may import more than Foundation.
//
// What the person types and where they point is a secret. Nothing here logs it, and a failure is
// told without it.

import Foundation

// MARK: - Places on the screen

/// A place on the screen in points, measured from the top left corner of the main display with y
/// growing downward: the space that the window server and the events of a Mac use.
struct ScreenPoint: Equatable {
  var x: Double
  var y: Double
}

/// A rectangle in that space. The picture of a window has one of its own, in pixels.
struct ScreenRect: Equatable {
  var x: Double
  var y: Double
  var width: Double
  var height: Double

  static let zero = ScreenRect(x: 0, y: 0, width: 0, height: 0)

  var right: Double { x + width }
  var bottom: Double { y + height }

  /// Whether it can hold something: its numbers are finite and it has a size above zero.
  var isUsable: Bool {
    x.isFinite && y.isFinite && width.isFinite && height.isFinite && width > 0 && height > 0
  }

  /// Whether the two share some area. Two that only touch along an edge do not.
  func overlaps(_ other: ScreenRect) -> Bool {
    x < other.right && other.x < right && y < other.bottom && other.y < bottom
  }
}

// MARK: - What the Rust side asks about a window

/// Which window has the keyboard, as far as the Mac can tell.
enum KeyboardFocus: Equatable {
  /// Another window has it, or the application of the shared window is not in front.
  case elsewhere
  /// The application of the window is in front and the window is its focus.
  case shared
  /// The Mac cannot tell, which counts as not the window.
  case unknown
}

/// A window in front of the shared one.
struct WindowFront: Equatable {
  var rect: ScreenRect
  /// It belongs to the application of the shared window.
  var sameApplication: Bool
  /// It is on the layer of an open menu.
  var menu: Bool
}

/// What the Mac is doing around the shared window at the moment of the call.
struct WindowSurroundings: Equatable {
  /// The Mac is locked or its display is off.
  var locked: Bool
  /// The window is on the screen, and not minimized, closed or gone.
  var onScreen: Bool
  var keyboard: KeyboardFocus
  /// Where the window is now.
  var window: ScreenRect
  /// The windows in front of it that draw something and can be pointed at or take keys.
  var fronts: [WindowFront]
}

// MARK: - What the window server and the system are asked

/// What the window server says of one window.
struct ListedWindow: Equatable {
  var id: UInt32
  /// The process that owns it.
  var process: Int32
  /// The layer it is on: 0 for the windows of applications, and more for the Dock, the menu bar,
  /// open menus and the like.
  var layer: Int
  /// 0 for a window that is not seen at all, 1 for one that is not see-through.
  var alpha: Double
  var bounds: ScreenRect
  var onScreen: Bool
}

/// What the window server says of one window that was asked about.
enum DescribedWindow: Equatable {
  case window(ListedWindow)
  /// The window server has no such window: it is closed, or it never was.
  case gone
  /// The window server could not be asked.
  case unavailable
}

protocol WindowListing: AnyObject {
  /// What the window server says of the window with this number, whether or not it is on the
  /// screen.
  func describe(window id: UInt32) -> DescribedWindow

  /// The windows that are on the screen and in front of the window with this number, in any order.
  /// nil if the window server could not be asked.
  func windowsOnScreen(inFrontOf id: UInt32) -> [ListedWindow]?
}

protocol MacActivity: AnyObject {
  /// Whether the Mac is locked or its display is off.
  var locked: Bool { get }

  /// The process of the application that is in front, if the system says.
  var frontmostProcess: Int32? { get }

  /// Brings the application of this process to the front, and returns when it is there, or when
  /// it is clear that it will not be soon. It waits for no longer than `ActivationWait.total`.
  func bringToFront(process: Int32)
}

/// How long the first touch waits for the application of the window to come to the front, in
/// seconds. The thread that puts the events to the window waits with it, and the rules of the Rust
/// side judge the events that wait behind it by how long ago their picture went out of date
/// (`MAX_STALE` in crates/baton-stream/src/gate.rs, 2 s), so the waits together stay well below
/// that. An application that is not in front yet when they are over is brought on by the system,
/// and a touch that came too early is the one that is dropped.
enum ActivationWait {
  /// For the main thread to carry out the request to activate the application.
  static let mainThread: TimeInterval = 0.25
  /// For the application to be in front after that.
  static let afterActivating: TimeInterval = 0.15
  /// For the system to open the application, which is the way that is not held to the
  /// cooperation between applications, when the request was not enough.
  static let opening: TimeInterval = 0.4
  /// For the application to be in front after it was opened.
  static let afterOpening: TimeInterval = 0.2

  /// All of them, as the most that a call can wait.
  static var total: TimeInterval {
    mainThread + afterActivating + opening + afterOpening
  }
}

/// The layers of the window server that tell what a window is. The numbers are the system's, and a
/// Mac reads them from the system (BatonControl.swift).
struct WindowLayers: Equatable {
  /// The layer of an open menu.
  var menu: Int
  /// The layers of the windows that can hold the keyboard focus of their application.
  var keys: ClosedRange<Int>
  /// From this layer up a window covers nothing and takes no keys: the cursor is drawn there.
  var untouchable: Int
}

// MARK: - The surroundings of a window

/// Where the surroundings of what is shared come from: a window, or a whole display.
protocol SurroundingsSource: AnyObject {
  /// nil if the system could not be asked.
  func read() -> WindowSurroundings?
}

/// Reads the surroundings of one window from the window server and from the system.
///
/// The system's Accessibility interface would say which window of an application has the
/// keyboard, but an app in the sandbox cannot use it for the windows of others. So the keyboard is
/// judged from what the window server lists: the application is in front, no window of it that
/// can hold the focus lies in front of the shared one, and no open menu of another application
/// takes the keys. It is a judgement, and it errs on the side of not typing.
final class SurroundingsReader: SurroundingsSource {
  /// The most windows in front that are given at once. A Mac with more says nothing, and what is
  /// put to the window is dropped.
  static let mostFronts = 128

  /// Windows that are fainter than this draw nothing that the person could point at.
  static let faintest = 0.01

  private let window: UInt32
  private let process: Int32
  private let listing: WindowListing
  private let activity: MacActivity
  private let layers: WindowLayers
  private let mostFronts: Int
  /// Whether the application has been brought to the front, which is done once.
  private var brought = false

  init(
    window: UInt32, process: Int32, listing: WindowListing, activity: MacActivity,
    layers: WindowLayers, mostFronts: Int = SurroundingsReader.mostFronts
  ) {
    self.window = window
    self.process = process
    self.listing = listing
    self.activity = activity
    self.layers = layers
    self.mostFronts = mostFronts
  }

  /// nil if the window server could not be asked, or there are more windows in front than the Rust
  /// side takes.
  func read() -> WindowSurroundings? {
    if activity.locked {
      // Nothing is put to a window while the Mac is locked, so there is no need to ask about it.
      return WindowSurroundings(
        locked: true, onScreen: false, keyboard: .unknown, window: .zero, fronts: [])
    }
    let own: ListedWindow
    switch listing.describe(window: window) {
    case .unavailable:
      return nil
    case .gone:
      return notOnScreen(at: .zero)
    case .window(let listed):
      own = listed
    }
    guard own.onScreen, own.bounds.isUsable else {
      return notOnScreen(at: own.bounds.isUsable ? own.bounds : .zero)
    }
    // Only a window that is on the screen is worth bringing the application forward for, and the
    // windows in front are read after it, since bringing it forward changes them. A touch on a
    // window that the person at the Mac minimized or hid, or that lies on another desktop, must not
    // show it again or switch their desktop: the session is paused then, and the touch is dropped.
    bringToFrontOnce()
    guard let above = listing.windowsOnScreen(inFrontOf: window) else { return nil }

    var fronts: [WindowFront] = []
    // A window of the application that can hold the focus lies in front of the shared one: it is
    // the likelier place for the keys, whether or not it overlaps.
    var applicationInFront = false
    // An open menu of another application takes the keys, and the application of the shared window
    // stays the one in front.
    var otherMenu = false
    for other in above where other.id != window {
      guard other.alpha > SurroundingsReader.faintest, other.bounds.isUsable,
        other.layer < layers.untouchable
      else { continue }
      let same = other.process == process
      let menu = other.layer == layers.menu
      if same && layers.keys.contains(other.layer) { applicationInFront = true }
      if menu && !same { otherMenu = true }
      // A menu counts wherever it is, since it takes the keys; any other window only if it can
      // lie over a point of the shared one.
      if menu || other.bounds.overlaps(own.bounds) {
        fronts.append(WindowFront(rect: other.bounds, sameApplication: same, menu: menu))
      }
    }
    if fronts.count > mostFronts { return nil }

    let keyboard: KeyboardFocus
    if let front = activity.frontmostProcess {
      keyboard = front == process && !applicationInFront && !otherMenu ? .shared : .elsewhere
    } else {
      keyboard = .unknown
    }
    return WindowSurroundings(
      locked: false, onScreen: true, keyboard: keyboard, window: own.bounds, fronts: fronts)
  }

  private func notOnScreen(at place: ScreenRect) -> WindowSurroundings {
    WindowSurroundings(
      locked: false, onScreen: false, keyboard: .unknown, window: place, fronts: [])
  }

  /// The person at the other device touches a window that is shared, and the application must be
  /// in front for the window to take keys. It is brought there once, when the first touch comes
  /// for a window that is on the screen, and not again: the person at the Mac may bring another one
  /// forward, and is not fought for it.
  private func bringToFrontOnce() {
    if brought { return }
    brought = true
    guard let front = activity.frontmostProcess, front != process else { return }
    activity.bringToFront(process: process)
  }
}

// MARK: - The surroundings of a whole display

protocol DisplayListing: AnyObject {
  /// Where the display with this number is, in points, if it is online and the system says.
  func bounds(ofDisplay id: UInt32) -> ScreenRect?
}

/// Reads the surroundings of a whole display. Everything on a display is shared, so no window lies
/// in front of it and none has the keyboard apart from it: a pointer reaches what the person at the
/// Mac would reach there, and a key goes to whatever has the keyboard. The rules on the Rust side
/// take that view of a display whatever is said here (`Surroundings::of_display` in
/// crates/baton-stream-abi/src/platform.rs); this says what the Mac is doing, which is whether it is
/// locked and where the display is.
final class DisplayReader: SurroundingsSource {
  private let display: UInt32
  private let listing: DisplayListing
  private let activity: MacActivity
  /// Where the display was when the Mac last said. A Mac that is locked, or a display that is
  /// gone, has no place to say, and a button that is let go of then is let go of at the place that
  /// the display had, and not in the corner of the main display, where a hot corner may be.
  private var lastPlace = ScreenRect.zero

  init(display: UInt32, listing: DisplayListing, activity: MacActivity) {
    self.display = display
    self.listing = listing
    self.activity = activity
  }

  func read() -> WindowSurroundings? {
    if activity.locked {
      // Nothing is put to a display while the Mac is locked, so there is no need to ask about it.
      return WindowSurroundings(
        locked: true, onScreen: false, keyboard: .unknown, window: lastPlace, fronts: [])
    }
    guard let bounds = listing.bounds(ofDisplay: display) else {
      // The display is not there any more: it was unplugged, or the Mac does not show it.
      return WindowSurroundings(
        locked: false, onScreen: false, keyboard: .unknown, window: lastPlace, fronts: [])
    }
    lastPlace = bounds
    return WindowSurroundings(
      locked: false, onScreen: true, keyboard: .shared, window: bounds, fronts: [])
  }
}

// MARK: - Pausing while the window cannot be seen

/// Why the pictures of a window are stopped.
enum PauseReason: Equatable {
  /// The Mac is locked or its display is off.
  case locked
  /// The window is minimized, on another desktop, or its application is hidden.
  case hidden
}

/// What the pause watch looks at.
struct WatchedState: Equatable {
  var locked: Bool
  var hidden: Bool
}

/// What there is to see about a window, or about a whole display, which has no number and is
/// never hidden. nil if the window server could not be asked, in which case nothing changes.
func watchedState(window: UInt32?, listing: WindowListing, activity: MacActivity) -> WatchedState? {
  if activity.locked { return WatchedState(locked: true, hidden: false) }
  guard let window else { return WatchedState(locked: false, hidden: false) }
  switch listing.describe(window: window) {
  case .unavailable:
    return nil
  case .gone:
    return WatchedState(locked: false, hidden: true)
  case .window(let listed):
    return WatchedState(locked: false, hidden: !(listed.onScreen && listed.bounds.isUsable))
  }
}

/// Pauses the session while the Mac is locked or the window is hidden, and resumes it when neither
/// is so any more. It is asked on a clock and tells the library only when the answer changes.
final class PauseWatch {
  private let session: SourceSession
  private let look: () -> WatchedState?
  private(set) var paused: PauseReason?

  init(session: SourceSession, look: @escaping () -> WatchedState?) {
    self.session = session
    self.look = look
  }

  func poll() {
    guard let seen = look() else { return }
    let wanted: PauseReason? = seen.locked ? .locked : (seen.hidden ? .hidden : nil)
    if wanted == paused { return }
    paused = wanted
    if let wanted {
      session.pause(wanted)
    } else {
      session.resume()
    }
  }
}

// MARK: - The geometry of a picture

/// How a window was laid out in a picture, which turns a place in the picture into a place on the
/// screen when the person at the other device touches it.
struct PictureGeometry: Equatable {
  /// The size of the window on the screen, in points.
  var windowWidth: Double
  var windowHeight: Double
  /// The size of the picture in pixels.
  var pictureWidth: UInt32
  var pictureHeight: UInt32
  /// Where the window is in the picture, in pixels. The rest of the picture is margin.
  var content: ScreenRect
}

/// The geometry of a picture of a window, or nil if something is not known or does not fit: a
/// picture without a geometry is sent all the same, and what the person touches on it is dropped.
/// `window` is the window as it was when the picture was captured, `content` is where the capture
/// put it in the picture.
func pictureGeometry(
  window: ScreenRect?, content: ScreenRect?, pictureWidth: UInt32, pictureHeight: UInt32
) -> PictureGeometry? {
  guard let window, window.isUsable, let content, content.isUsable, pictureWidth >= 1,
    pictureHeight >= 1
  else { return nil }
  let width = Double(pictureWidth)
  let height = Double(pictureHeight)
  // A pixel of slack for rounding, but not a window that is somewhere else than in this picture.
  let slack = 1.0
  guard content.x >= -slack, content.y >= -slack, content.right <= width + slack,
    content.bottom <= height + slack
  else { return nil }
  // The capture scales a window to fit the picture and keeps its shape, so it is drawn at the
  // scale that makes it fill the picture on one axis at least. A rectangle of another size is not
  // where the window is, or is not in the units that are taken for it (points for pixels), and
  // what the person touched would land elsewhere than where they pointed.
  let fit = min(width / window.width, height / window.height)
  let rounding = 2.0
  guard abs(content.width - window.width * fit) <= rounding,
    abs(content.height - window.height * fit) <= rounding
  else { return nil }
  let left = max(0, content.x)
  let top = max(0, content.y)
  let inside = ScreenRect(
    x: left, y: top, width: min(content.right, width) - left,
    height: min(content.bottom, height) - top)
  guard inside.isUsable else { return nil }
  return PictureGeometry(
    windowWidth: window.width, windowHeight: window.height, pictureWidth: pictureWidth,
    pictureHeight: pictureHeight, content: inside)
}

/// How the layout of the images of a capture was found, which `CaptureLayout` tells when that
/// changes.
enum LayoutSource: Equatable {
  /// From what the capture says of the image.
  case reported
  /// The capture said nothing that fits, and the window has the shape of the picture, which it
  /// fills.
  case wholePicture
  /// Nothing fits: the images go out with no geometry, and what the person touches is dropped.
  case none
}

/// Works out the geometry of each image that the capture of a window, or of a whole display, makes:
/// the size of the window on the screen comes from the window server, and of the display from the
/// system, and where the capture put it in the picture from what the capture says of the image.
/// What it says is believed only when it fits what the capture is set up to do
/// (`pictureGeometry`). When it says nothing, or something that does not fit, what has the shape of
/// the picture is taken to fill it, since that is what scaling it to fit does. Anything else has no
/// geometry.
///
/// It is used by one thread, the one that the images arrive on.
final class CaptureLayout {
  /// How long a reading of the system is used again, in seconds: one reading serves the many
  /// images of a window that is being drawn on.
  static let reuse: TimeInterval = 0.05

  private let noun: String
  private let bounds: () -> ScreenRect?
  private let now: () -> TimeInterval
  private let note: (String) -> Void
  private var reading: (at: TimeInterval, bounds: ScreenRect?)?
  private var found: LayoutSource?

  /// `bounds` says where what is shared is on the screen, in points, and nil if it cannot say.
  /// `note` is told, with a sentence that holds sizes and nothing else, whenever the way the layout
  /// was found changes, and the first time; `noun` is what the sentence calls what is shared.
  init(
    noun: String, bounds: @escaping () -> ScreenRect?,
    now: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime },
    note: @escaping (String) -> Void
  ) {
    self.noun = noun
    self.bounds = bounds
    self.now = now
    self.note = note
  }

  /// The layout of the images of a window, which the window server says the place of.
  convenience init(
    window: UInt32, listing: WindowListing,
    now: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime },
    note: @escaping (String) -> Void
  ) {
    self.init(
      noun: "window",
      bounds: {
        switch listing.describe(window: window) {
        case .window(let listed): return listed.bounds
        case .gone, .unavailable: return nil
        }
      }, now: now, note: note)
  }

  /// The geometry of an image of `pictureWidth` by `pictureHeight` pixels, where `reported` is the
  /// rectangle that the capture says the window has in it, in pixels.
  func geometry(pictureWidth: UInt32, pictureHeight: UInt32, reported: ScreenRect?)
    -> PictureGeometry?
  {
    let size = sharedRect()
    let source: LayoutSource
    let geometry: PictureGeometry?
    if let said = pictureGeometry(
      window: size, content: reported, pictureWidth: pictureWidth, pictureHeight: pictureHeight)
    {
      (source, geometry) = (.reported, said)
    } else if let whole = pictureGeometry(
      window: size,
      content: ScreenRect(x: 0, y: 0, width: Double(pictureWidth), height: Double(pictureHeight)),
      pictureWidth: pictureWidth, pictureHeight: pictureHeight)
    {
      (source, geometry) = (.wholePicture, whole)
    } else {
      (source, geometry) = (.none, nil)
    }
    if source != found {
      found = source
      note(
        "the layout of the images is \(source) (\(noun) \(describe(size)) points, picture "
          + "\(pictureWidth)x\(pictureHeight) pixels, capture says \(describe(reported)))")
    }
    return geometry
  }

  private func sharedRect() -> ScreenRect? {
    let time = now()
    if let reading, time >= reading.at, time - reading.at < CaptureLayout.reuse {
      return reading.bounds
    }
    let rect = bounds()
    reading = (time, rect)
    return rect
  }

  private func describe(_ rect: ScreenRect?) -> String {
    guard let rect else { return "nothing" }
    func number(_ value: Double) -> String { String(format: "%.1f", value) }
    return "\(number(rect.x)),\(number(rect.y)) \(number(rect.width))x\(number(rect.height))"
  }
}

/// The rectangle that a dictionary holds in the form that the system gives one in: the bounds that
/// the window server says a window has, and the place that the capture says a window has in an
/// image. nil if something is missing or is not a number.
func screenRect(fromDictionary value: Any?) -> ScreenRect? {
  guard let dictionary = value as? [String: Any],
    let x = (dictionary["X"] as? NSNumber)?.doubleValue,
    let y = (dictionary["Y"] as? NSNumber)?.doubleValue,
    let width = (dictionary["Width"] as? NSNumber)?.doubleValue,
    let height = (dictionary["Height"] as? NSNumber)?.doubleValue
  else { return nil }
  return ScreenRect(x: x, y: y, width: width, height: height)
}

// MARK: - What the person did at the other device

enum PointerAction: Equatable {
  /// It moved, with a button down or not.
  case move
  case down
  case up
  /// A scroll by `dx` and `dy` points: a positive `dx` moves the content to the right, and a
  /// positive `dy` moves it down, as a finger that drags the content does.
  case scroll
}

enum PointerButton: Equatable {
  /// The primary button, or a touch.
  case left
  case right
  case middle
}

/// The modifier keys that are down. The values are those of BATON_MOD_ in the C interface.
struct InputModifiers: OptionSet, Equatable {
  let rawValue: UInt32
  static let shift = InputModifiers(rawValue: 1)
  static let control = InputModifiers(rawValue: 2)
  static let option = InputModifiers(rawValue: 4)
  static let command = InputModifiers(rawValue: 8)
  static let capsLock = InputModifiers(rawValue: 16)
}

/// The flags that the keyboard of a Mac sets on some keys besides the modifiers that are down, and
/// that the shortcuts of the system look for: the function flag on the arrows, the function keys
/// and the keys above the arrows, and the flag of the numeric keypad on the arrows too (the
/// documentation of `NSEvent.ModifierFlags` says so of both). Control and an arrow, which opens
/// Mission Control or changes the Space, is probably not that shortcut for the system without
/// them (inferred, R6 checks it).
struct KeyFlags: OptionSet, Equatable {
  let rawValue: UInt8
  static let function = KeyFlags(rawValue: 1)
  static let numericPad = KeyFlags(rawValue: 2)
}

struct PointerPost: Equatable {
  /// A place on the screen, in points.
  var at: ScreenPoint
  var action: PointerAction
  var button: PointerButton
  var dx: Double
  var dy: Double
  var modifiers: InputModifiers
}

struct KeyPost: Equatable {
  /// A USB HID usage of the keyboard page, or 0 for text that came with no key.
  var code: UInt32
  var down: Bool
  /// The system repeats a key that is held.
  var repeating: Bool
  var modifiers: InputModifiers
  /// What the key produced, if anything.
  var text: String?
}

/// One thing that the person did, put to the window.
enum InputPost: Equatable {
  case pointer(PointerPost)
  case key(KeyPost)
}

// MARK: - The events of a Mac

enum MouseKind: Equatable {
  case moved
  case down
  case up
  case dragged
}

struct SyntheticMouse: Equatable {
  var kind: MouseKind
  /// The button that went down or up, or is held in a drag.
  var button: PointerButton
  var at: ScreenPoint
  /// 1 for a click, 2 for a double click, 3 for a triple click, and 0 for a move with no button.
  var clickCount: Int
  /// 0 to 1: 0 for a button that is up.
  var pressure: Double
  var modifiers: InputModifiers
}

/// A scroll in pixels, as a trackpad reports it.
struct SyntheticScroll: Equatable {
  var at: ScreenPoint
  var dx: Int32
  var dy: Int32
  var modifiers: InputModifiers
}

struct SyntheticKey: Equatable {
  /// The virtual key code of the Mac (kVK_ in HIToolbox/Events.h).
  var virtualKey: UInt16
  var down: Bool
  var autorepeat: Bool
  var modifiers: InputModifiers
  /// What the keyboard of a Mac sets on this key besides the modifiers.
  var keyFlags: KeyFlags = []
  /// The UTF-16 units to type, for an event that has no key.
  var text: [UInt16]?
}

enum SyntheticEvent: Equatable {
  case mouse(SyntheticMouse)
  case scroll(SyntheticScroll)
  case key(SyntheticKey)
}

/// The usages of the keyboard page for the modifier keys. They are not events of their own: the
/// modifiers that are down come with every event, and a Mac makes a key from them.
func isModifierUsage(_ usage: UInt32) -> Bool {
  (0xE0...0xE7).contains(usage)
}

/// The virtual key codes of the Mac for the USB HID usages of the keyboard page (0x07), which is
/// what the viewers have. The usages are those of the HID Usage Tables, and the codes are the
/// kVK_ constants of HIToolbox/Events.h. They stand for the position of a key, as the usages do,
/// so the layout of the Mac decides what the key types.
let macKeysByUsage: [UInt32: UInt16] = [
  // Letters, a to z.
  0x04: 0x00, 0x05: 0x0B, 0x06: 0x08, 0x07: 0x02, 0x08: 0x0E, 0x09: 0x03, 0x0A: 0x05,
  0x0B: 0x04, 0x0C: 0x22, 0x0D: 0x26, 0x0E: 0x28, 0x0F: 0x25, 0x10: 0x2E, 0x11: 0x2D,
  0x12: 0x1F, 0x13: 0x23, 0x14: 0x0C, 0x15: 0x0F, 0x16: 0x01, 0x17: 0x11, 0x18: 0x20,
  0x19: 0x09, 0x1A: 0x0D, 0x1B: 0x07, 0x1C: 0x10, 0x1D: 0x06,
  // Digits, 1 to 9 and 0.
  0x1E: 0x12, 0x1F: 0x13, 0x20: 0x14, 0x21: 0x15, 0x22: 0x17, 0x23: 0x16, 0x24: 0x1A,
  0x25: 0x1C, 0x26: 0x19, 0x27: 0x1D,
  // Return, Escape, Delete (backspace), Tab, Space.
  0x28: 0x24, 0x29: 0x35, 0x2A: 0x33, 0x2B: 0x30, 0x2C: 0x31,
  // - = [ ] \ ; ' ` , . / and Caps Lock. (The key left of Return on an ISO keyboard, usage 0x32,
  // has the code of the backslash on a Mac, which the usage 0x31 has already.)
  0x2D: 0x1B, 0x2E: 0x18, 0x2F: 0x21, 0x30: 0x1E, 0x31: 0x2A, 0x33: 0x29, 0x34: 0x27,
  0x35: 0x32, 0x36: 0x2B, 0x37: 0x2F, 0x38: 0x2C, 0x39: 0x39,
  // F1 to F12.
  0x3A: 0x7A, 0x3B: 0x78, 0x3C: 0x63, 0x3D: 0x76, 0x3E: 0x60, 0x3F: 0x61, 0x40: 0x62,
  0x41: 0x64, 0x42: 0x65, 0x43: 0x6D, 0x44: 0x67, 0x45: 0x6F,
  // Insert (Help on a Mac), Home, Page Up, Delete forward, End, Page Down, and the arrows.
  0x49: 0x72, 0x4A: 0x73, 0x4B: 0x74, 0x4C: 0x75, 0x4D: 0x77, 0x4E: 0x79, 0x4F: 0x7C,
  0x50: 0x7B, 0x51: 0x7D, 0x52: 0x7E,
  // The keypad: Num Lock (Clear on a Mac), / * - + Enter, 1 to 9, 0, the point, and =.
  0x53: 0x47, 0x54: 0x4B, 0x55: 0x43, 0x56: 0x4E, 0x57: 0x45, 0x58: 0x4C, 0x59: 0x53,
  0x5A: 0x54, 0x5B: 0x55, 0x5C: 0x56, 0x5D: 0x57, 0x5E: 0x58, 0x5F: 0x59, 0x60: 0x5B,
  0x61: 0x5C, 0x62: 0x52, 0x63: 0x41, 0x67: 0x51,
  // The key beside the left Shift of an ISO keyboard, the menu key, and F13 to F20.
  0x64: 0x0A, 0x65: 0x6E, 0x68: 0x69, 0x69: 0x6B, 0x6A: 0x71, 0x6B: 0x6A, 0x6C: 0x40,
  0x6D: 0x4F, 0x6E: 0x50, 0x6F: 0x5A,
]

/// The virtual key code of the Mac for a usage of the keyboard page, or nil if there is none.
func macKey(forUsage usage: UInt32) -> UInt16? {
  macKeysByUsage[usage]
}

/// The virtual key codes of the four arrows (kVK_LeftArrow, kVK_RightArrow, kVK_DownArrow and
/// kVK_UpArrow).
private let arrowKeys: Set<UInt16> = [0x7B, 0x7C, 0x7D, 0x7E]

/// The virtual key codes of the other keys that have the function flag on the keyboard of a Mac:
/// F1 to F20, and the six keys above the arrows (Help, Home, Page Up, forward delete, End and Page
/// Down).
private let functionKeys: Set<UInt16> = [
  0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F,
  0x69, 0x6B, 0x71, 0x6A, 0x40, 0x4F, 0x50, 0x5A,
  0x72, 0x73, 0x74, 0x75, 0x77, 0x79,
]

/// What the keyboard of a Mac sets on the key with this virtual key code besides the modifiers
/// that are down.
func keyFlags(forVirtualKey key: UInt16) -> KeyFlags {
  if arrowKeys.contains(key) { return [.function, .numericPad] }
  return functionKeys.contains(key) ? [.function] : []
}

/// The shortcuts that a Mac keeps for itself, by the virtual key code and the modifiers that make
/// them: the system acts on them for the whole Mac before the application in front sees them.
/// They are the defaults of the system (System Settings, Keyboard, Keyboard Shortcuts, and the
/// ones that the application switcher, Force Quit and the log-out dialog have), so a Mac whose
/// shortcuts were changed may react to other keys. A key has the shortcut when it is held with at
/// least these modifiers.
private let systemShortcuts: [(key: UInt16, needs: InputModifiers)] = [
  // Spotlight and the Finder search (Command Space): they take the keyboard without being the
  // application in front, so what is typed next would go to them.
  (0x31, .command),
  // The application switcher (Command Tab, and with Shift the other way).
  (0x30, .command),
  // Force Quit (Command Option Escape).
  (0x35, [.command, .option]),
  // Lock Screen (Control Command Q), and Log Out (Command Shift Q).
  (0x0C, [.control, .command]), (0x0C, [.command, .shift]),
  // Hide or show the Dock (Command Option D), and hide the other applications (Command Option H).
  (0x02, [.command, .option]), (0x04, [.command, .option]),
  // The screenshots (Command Shift 3, 4, 5 and 6): they can go to the clipboard, and from there
  // into the window, with the rest of the screen in them.
  (0x14, [.command, .shift]), (0x15, [.command, .shift]), (0x17, [.command, .shift]),
  (0x16, [.command, .shift]),
  // Mission Control, the windows of the application, and the spaces (Control with an arrow).
  (0x7B, .control), (0x7C, .control), (0x7D, .control), (0x7E, .control),
  // Keyboard navigation (Control with F1 to F8): it turns keyboard access on or off (F1, F7) and
  // takes the keys to the menu bar (F2), the Dock (F3), the next window (F4), the toolbar (F5),
  // the next panel (F6) and the menus of the status items (F8), where what is typed next acts on
  // the whole Mac, such as the Apple menu with its Lock Screen and Shut Down. F3 and F4 are listed
  // here as well, so that they stay if the entries for those keys alone are narrowed.
  (0x7A, .control), (0x78, .control), (0x63, .control), (0x76, .control),
  (0x60, .control), (0x61, .control), (0x62, .control), (0x64, .control),
  // Mission Control (F3), Launchpad and Spotlight (F4), Show Desktop (F11), and VoiceOver
  // (Command F5).
  (0x63, []), (0x76, []), (0x67, []), (0x60, .command),
]

/// Whether the key, held with these modifiers, is a shortcut that a Mac keeps for itself.
func isSystemShortcut(virtualKey key: UInt16, modifiers: InputModifiers) -> Bool {
  systemShortcuts.contains { $0.key == key && modifiers.isSuperset(of: $0.needs) }
}

/// Turns what the person did into the events of a Mac, and keeps what it takes to do so: which
/// buttons are down, how many clicks in a row there were, the part of a scroll that is less than a
/// pixel, and where the pointer was put. It is used by one thread at a time.
final class InputTranslator {
  /// The most UTF-16 units of text that one event carries, as much as one call of the system keeps.
  static let mostTextUnits = 20
  /// The most pixels that a scroll event moves by, as the protocol allows.
  static let mostScroll = 10_000.0

  private let doubleClickInterval: TimeInterval
  private let clickSlop: Double
  private let movesPointerToScroll: Bool
  private let dropsSystemShortcuts: Bool
  private let now: () -> TimeInterval

  private var held: [PointerButton] = []
  private var lastPress: (button: PointerButton, time: TimeInterval, at: ScreenPoint, count: Int)?
  private var restAcross = 0.0
  private var restDown = 0.0
  /// The keys whose press was dropped as a shortcut of the system, so that their release is too.
  private var droppedKeys: Set<UInt16> = []
  /// The keys whose press was posted and that are not let go of yet. A repeat of one of them that
  /// comes with the modifiers of a shortcut is left out, and its release is not, since the press
  /// went to the Mac.
  private var pressedKeys: Set<UInt16> = []

  /// `doubleClickInterval` is the time in which a second press makes a double click, and
  /// `clickSlop` how far in points it may be from the first. `movesPointerToScroll` is for events
  /// that go to the system: it takes a scroll to the place of the pointer, and not to the place
  /// that the event names (inferred), so the pointer is moved to the place of every scroll first
  /// (where it is cannot be known: the person at this Mac has a pointer of their own, and a post
  /// can fail). `dropsSystemShortcuts` is for a window, whose events go to the system as well:
  /// the shortcuts that the system keeps for the whole Mac are not let through, as they would take
  /// the person out of the window. `now` is a clock in seconds.
  init(
    doubleClickInterval: TimeInterval = 0.5, clickSlop: Double = 4,
    movesPointerToScroll: Bool = false, dropsSystemShortcuts: Bool = false,
    now: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }
  ) {
    self.doubleClickInterval = doubleClickInterval
    self.clickSlop = clickSlop
    self.movesPointerToScroll = movesPointerToScroll
    self.dropsSystemShortcuts = dropsSystemShortcuts
    self.now = now
  }

  /// The events to make of a post: none for one that makes nothing (a modifier key, whose state
  /// comes with the other events, or a scroll of less than a pixel), and nil for one that cannot be
  /// made into events at all (a key that has no code on a Mac, or a number that is not one).
  func events(for post: InputPost) -> [SyntheticEvent]? {
    switch post {
    case .pointer(let pointer): return events(for: pointer)
    case .key(let key): return events(for: key)
    }
  }

  // MARK: The pointer

  private func events(for post: PointerPost) -> [SyntheticEvent]? {
    guard post.at.x.isFinite, post.at.y.isFinite else { return nil }
    switch post.action {
    case .move:
      // A move with a button down is a drag with that button, and goes on with its click count.
      guard let button = held.first else {
        return [.mouse(mouse(.moved, post, button: post.button, count: 0, pressure: 0))]
      }
      return [
        .mouse(mouse(.dragged, post, button: button, count: pressCount(of: button), pressure: 1))
      ]
    case .down:
      let count = clicks(for: post)
      held.removeAll { $0 == post.button }
      held.append(post.button)
      return [.mouse(mouse(.down, post, button: post.button, count: count, pressure: 1))]
    case .up:
      held.removeAll { $0 == post.button }
      let count = pressCount(of: post.button)
      return [.mouse(mouse(.up, post, button: post.button, count: count, pressure: 0))]
    case .scroll:
      return scroll(post)
    }
  }

  /// The click count of the last press of this button, which its drags and its release carry.
  private func pressCount(of button: PointerButton) -> Int {
    if let last = lastPress, last.button == button { return last.count }
    return 1
  }

  private func mouse(
    _ kind: MouseKind, _ post: PointerPost, button: PointerButton, count: Int, pressure: Double
  ) -> SyntheticMouse {
    SyntheticMouse(
      kind: kind, button: button, at: post.at, clickCount: count, pressure: pressure,
      modifiers: post.modifiers)
  }

  /// How many presses of this button there have been in a row: a press that comes soon enough and
  /// close enough to the one before it, of the same button, counts on from it.
  private func clicks(for post: PointerPost) -> Int {
    let time = now()
    var count = 1
    if let last = lastPress, last.button == post.button, time - last.time <= doubleClickInterval,
      time >= last.time, abs(post.at.x - last.at.x) <= clickSlop,
      abs(post.at.y - last.at.y) <= clickSlop
    {
      count = min(last.count + 1, 1000)
    }
    lastPress = (post.button, time, post.at, count)
    return count
  }

  private func scroll(_ post: PointerPost) -> [SyntheticEvent]? {
    guard post.dx.isFinite, post.dy.isFinite else { return nil }
    let across = whole(restAcross + post.dx)
    let down = whole(restDown + post.dy)
    restAcross = across.rest
    restDown = down.rest
    if across.pixels == 0 && down.pixels == 0 { return [] }
    let scrolled = SyntheticEvent.scroll(
      SyntheticScroll(at: post.at, dx: across.pixels, dy: down.pixels, modifiers: post.modifiers))
    guard movesPointerToScroll else { return [scrolled] }
    // The pointer goes to the scroll first, as it would for a move at that place.
    let move = PointerPost(
      at: post.at, action: .move, button: post.button, dx: 0, dy: 0, modifiers: post.modifiers)
    guard let moved = events(for: move) else { return nil }
    return moved + [scrolled]
  }

  /// The whole pixels of a distance, and what is left of it.
  private func whole(_ distance: Double) -> (pixels: Int32, rest: Double) {
    let limit = InputTranslator.mostScroll
    let clamped = max(-limit, min(limit, distance))
    let pixels = clamped.rounded(.towardZero)
    return (Int32(pixels), clamped - pixels)
  }

  // MARK: The keyboard

  private func events(for post: KeyPost) -> [SyntheticEvent]? {
    if post.code == 0 {
      return typed(post)
    }
    // A modifier key makes no event of its own.
    if isModifierUsage(post.code) { return [] }
    guard let key = macKey(forUsage: post.code) else { return nil }
    if dropsSystemShortcuts {
      if post.down {
        if isSystemShortcut(virtualKey: key, modifiers: post.modifiers) {
          // A key that went through and is held on, with a modifier going down while it repeats,
          // is released when it is let go of: only a press that did not go through is marked.
          if !pressedKeys.contains(key) { droppedKeys.insert(key) }
          return nil
        }
        // The key went down again with other modifiers: this press is one that goes through.
        droppedKeys.remove(key)
        pressedKeys.insert(key)
      } else if droppedKeys.remove(key) != nil {
        // The press of this key was dropped, so there is nothing to release.
        return []
      } else {
        pressedKeys.remove(key)
      }
    }
    return [
      .key(
        SyntheticKey(
          virtualKey: key, down: post.down, autorepeat: post.down && post.repeating,
          modifiers: post.modifiers, keyFlags: keyFlags(forVirtualKey: key), text: nil))
    ]
  }

  /// Text that came with no key goes down and up once, and is never held.
  private func typed(_ post: KeyPost) -> [SyntheticEvent]? {
    // The release of text is not told, and one that comes is not a second text.
    if !post.down { return [] }
    guard let text = post.text, !text.isEmpty else { return nil }
    let units = Array(text.utf16)
    guard units.count <= InputTranslator.mostTextUnits else { return nil }
    return [true, false].map { down in
      SyntheticEvent.key(
        SyntheticKey(
          virtualKey: 0, down: down, autorepeat: false, modifiers: post.modifiers, text: units))
    }
  }
}

// MARK: - The consent to steer a window or a display

/// Whether this Mac lets the app hand input events to the windows of other applications and to the
/// system, which a window or a screen needs to be shared with control. The person gives the consent
/// once, in the settings of the system, and can take it back at any time. The raw values are the
/// answers that the app's screens read (apps/baton/lib/src/control_access.dart).
enum ControlAccess: String, Equatable {
  /// The person has let the app.
  case allowed
  /// The system could let the app, and the person has not.
  case needed
  /// This Mac cannot have a window or a screen steered: its system is too old to say which the
  /// person picked, or it shares nothing at all.
  case unavailable
}

// MARK: - A window or a display that is steered

/// Hands events to the system, as if the person at the Mac did them (BatonControl.swift has the one
/// of a Mac).
protocol EventPoster: AnyObject {
  /// Hands the event over. false if the event could not be made, or the system does not let this
  /// app hand it over.
  func post(_ event: SyntheticEvent) -> Bool
}

/// What the Rust side asks of a window, or a whole display, that is shared with control: its
/// surroundings, and to have something that the person did put to it.
final class WindowControl {
  private let reader: SurroundingsSource
  private let translator: InputTranslator
  private let poster: EventPoster

  init(reader: SurroundingsSource, translator: InputTranslator, poster: EventPoster) {
    self.reader = reader
    self.translator = translator
    self.poster = poster
  }

  func surroundings() -> WindowSurroundings? {
    reader.read()
  }

  /// false if nothing could be put to the window, or only some of it.
  func post(_ post: InputPost) -> Bool {
    guard let events = translator.events(for: post) else { return false }
    for event in events where !poster.post(event) {
      return false
    }
    return true
  }
}
