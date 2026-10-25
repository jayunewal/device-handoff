// SPDX-License-Identifier: Apache-2.0

// Tests of BatonInput.swift, and of the parts of BatonSource.swift that carry input: the two
// functions of the table that the Rust side calls to ask about a window and to put something to
// it, the geometry of a picture, and the news of a change and a pause. The window server and the
// system are played by fakes.

import Foundation

// MARK: - What the system is played by

final class FakeListing: WindowListing {
  var described: DescribedWindow = .gone
  var inFront: [ListedWindow]? = []
  private(set) var describedIds: [UInt32] = []
  private(set) var frontIds: [UInt32] = []
  /// Called when the window is described, for a test that changes the world while it is read.
  var onDescribe: (() -> Void)?

  func describe(window id: UInt32) -> DescribedWindow {
    describedIds.append(id)
    onDescribe?()
    return described
  }

  func windowsOnScreen(inFrontOf id: UInt32) -> [ListedWindow]? {
    frontIds.append(id)
    return inFront
  }
}

final class FakeActivity: MacActivity {
  var locked = false
  var frontmostProcess: Int32? = sharedProcess
  private(set) var brought: [Int32] = []
  /// What happens when the application is brought to the front.
  var onBring: (() -> Void)?

  func bringToFront(process: Int32) {
    brought.append(process)
    onBring?()
  }
}

final class FakePoster: EventPoster {
  var posted: [SyntheticEvent] = []
  /// For each call, whether it works; calls beyond the list work.
  var works: [Bool] = []

  func post(_ event: SyntheticEvent) -> Bool {
    posted.append(event)
    return works.isEmpty ? true : works.removeFirst()
  }
}

/// What the system says of displays: where each is, if it says.
final class FakeDisplays: DisplayListing {
  var places: [UInt32: ScreenRect] = [:]
  private(set) var asked: [UInt32] = []

  func bounds(ofDisplay id: UInt32) -> ScreenRect? {
    asked.append(id)
    return places[id]
  }
}

let sharedDisplay: UInt32 = 3
let displayBounds = ScreenRect(x: 0, y: 0, width: 1512, height: 982)

let sharedWindow: UInt32 = 7
let sharedProcess: Int32 = 100
let sharedBounds = ScreenRect(x: 100, y: 100, width: 800, height: 600)
let testLayers = WindowLayers(menu: 101, keys: 0...8, untouchable: 2000)

func listed(
  _ id: UInt32, process: Int32 = 200, layer: Int = 0, alpha: Double = 1,
  bounds: ScreenRect = sharedBounds, onScreen: Bool = true
) -> ListedWindow {
  ListedWindow(
    id: id, process: process, layer: layer, alpha: alpha, bounds: bounds, onScreen: onScreen)
}

/// A reader of the shared window, number 7 of process 100, which is in front, on the screen, with
/// nothing in front of it.
func readerOfASharedWindow(
  mostFronts: Int = SurroundingsReader.mostFronts,
  _ setUp: (FakeListing, FakeActivity) -> Void = { _, _ in }
) -> (SurroundingsReader, FakeListing, FakeActivity) {
  let listing = FakeListing()
  listing.described = .window(listed(sharedWindow, process: sharedProcess))
  let activity = FakeActivity()
  setUp(listing, activity)
  let reader = SurroundingsReader(
    window: sharedWindow, process: sharedProcess, listing: listing, activity: activity,
    layers: testLayers, mostFronts: mostFronts)
  return (reader, listing, activity)
}

// MARK: - The surroundings of a window

func testTheSurroundingsOfAWindowInFrontAreOnTheScreenWithTheKeyboard() {
  let (reader, listing, activity) = readerOfASharedWindow()
  let around = reader.read()
  check(
    around
      == WindowSurroundings(
        locked: false, onScreen: true, keyboard: .shared, window: sharedBounds, fronts: []),
    "\(String(describing: around))")
  check(listing.describedIds == [sharedWindow] && listing.frontIds == [sharedWindow])
  check(activity.brought.isEmpty, "the application was in front already")
}

func testALockedMacIsToldWithoutAskingTheWindowServer() {
  let (reader, listing, activity) = readerOfASharedWindow { _, activity in
    activity.locked = true
    activity.frontmostProcess = 999
  }
  check(
    reader.read()
      == WindowSurroundings(
        locked: true, onScreen: false, keyboard: .unknown, window: .zero, fronts: []))
  check(listing.describedIds.isEmpty && listing.frontIds.isEmpty)
  check(activity.brought.isEmpty, "a locked Mac is not touched")
}

func testAWindowThatIsGoneOrNotOnTheScreenIsNotOnTheScreen() {
  let (gone, _, _) = readerOfASharedWindow { listing, _ in listing.described = .gone }
  check(
    gone.read()
      == WindowSurroundings(
        locked: false, onScreen: false, keyboard: .unknown, window: .zero, fronts: []))

  let (minimized, listing, _) = readerOfASharedWindow { listing, _ in
    listing.described = .window(listed(sharedWindow, process: sharedProcess, onScreen: false))
  }
  check(
    minimized.read()
      == WindowSurroundings(
        locked: false, onScreen: false, keyboard: .unknown, window: sharedBounds, fronts: []))
  check(listing.frontIds.isEmpty, "what lies in front of a window that is not seen is not asked")

  let empty = ScreenRect(x: 0, y: 0, width: 0, height: 600)
  let (flat, _, _) = readerOfASharedWindow { listing, _ in
    listing.described = .window(listed(sharedWindow, process: sharedProcess, bounds: empty))
  }
  check(
    flat.read()
      == WindowSurroundings(
        locked: false, onScreen: false, keyboard: .unknown, window: .zero, fronts: []))
}

func testAWindowServerThatCannotBeAskedMakesNoAnswer() {
  let (first, _, _) = readerOfASharedWindow { listing, _ in listing.described = .unavailable }
  check(first.read() == nil)
  let (second, _, _) = readerOfASharedWindow { listing, _ in listing.inFront = nil }
  check(second.read() == nil)
}

func testWindowsInFrontAreListedWhenTheyDrawSomethingAndCanLieOverTheWindow() {
  let overlapping = ScreenRect(x: 500, y: 300, width: 600, height: 500)
  let (reader, _, _) = readerOfASharedWindow { listing, _ in
    listing.inFront = [
      // Another application's window over the corner.
      listed(20, process: 300, bounds: overlapping),
      // A window of the same application over the left edge.
      listed(21, process: sharedProcess, layer: 20, bounds: ScreenRect(x: 0, y: 200, width: 150, height: 50)),
      // Not over the window at all, and one that only touches its edge.
      listed(22, bounds: ScreenRect(x: 1000, y: 100, width: 50, height: 50)),
      listed(23, bounds: ScreenRect(x: 900, y: 100, width: 50, height: 50)),
      // Not seen, and not a window.
      listed(24, alpha: 0, bounds: overlapping),
      listed(25, bounds: ScreenRect(x: 300, y: 300, width: 0, height: 50)),
      // See-through but there, and so faint that nothing of it is seen, to the limit and a little
      // above it.
      listed(26, alpha: 0.5, bounds: ScreenRect(x: 300, y: 300, width: 10, height: 10)),
      listed(29, alpha: SurroundingsReader.faintest, bounds: overlapping),
      listed(
        30, alpha: SurroundingsReader.faintest.nextUp,
        bounds: ScreenRect(x: 320, y: 320, width: 10, height: 10)),
      // From the layer of the cursor up.
      listed(27, layer: 2000, bounds: overlapping),
      listed(28, layer: 2_147_483_630, bounds: overlapping),
      // The shared window itself, were the system to list it.
      listed(sharedWindow, process: sharedProcess),
    ]
  }
  let around = reader.read()
  check(
    around?.fronts == [
      WindowFront(rect: overlapping, sameApplication: false, menu: false),
      WindowFront(
        rect: ScreenRect(x: 0, y: 200, width: 150, height: 50), sameApplication: true, menu: false),
      WindowFront(
        rect: ScreenRect(x: 300, y: 300, width: 10, height: 10), sameApplication: false,
        menu: false),
      WindowFront(
        rect: ScreenRect(x: 320, y: 320, width: 10, height: 10), sameApplication: false,
        menu: false),
    ], "\(String(describing: around?.fronts))")
}

func testAnOpenMenuCountsWhereverItIsAndOtherWindowsOnlyWhereTheyOverlap() {
  let faraway = ScreenRect(x: 1500, y: 0, width: 200, height: 300)
  let (reader, _, _) = readerOfASharedWindow { listing, _ in
    listing.inFront = [
      listed(30, process: 300, layer: 101, bounds: faraway),
      listed(31, process: sharedProcess, layer: 101, bounds: faraway),
      listed(32, process: 300, layer: 100, bounds: faraway),
    ]
  }
  let around = reader.read()
  check(
    around?.fronts == [
      WindowFront(rect: faraway, sameApplication: false, menu: true),
      WindowFront(rect: faraway, sameApplication: true, menu: true),
    ], "\(String(describing: around?.fronts))")
  // A menu of another application takes the keys, and a menu of the application is the business of
  // the Rust side, which sees it among the windows in front.
  check(around?.keyboard == .elsewhere)

  let (own, _, _) = readerOfASharedWindow { listing, _ in
    listing.inFront = [listed(31, process: sharedProcess, layer: 101, bounds: faraway)]
  }
  check(own.read()?.keyboard == .shared)
}

func testTheKeyboardIsTheWindowsOnlyWhileItsApplicationIsInFrontWithNoWindowOfItBefore() {
  let (elsewhere, _, _) = readerOfASharedWindow { _, activity in activity.frontmostProcess = 300 }
  check(elsewhere.read()?.keyboard == .elsewhere)

  let (unknown, _, _) = readerOfASharedWindow { _, activity in activity.frontmostProcess = nil }
  check(unknown.read()?.keyboard == .unknown)

  // Another window of the application that can hold the focus is the likelier place for the keys,
  // whether or not it lies over the shared one.
  let beside = ScreenRect(x: 1000, y: 100, width: 100, height: 100)
  for layer in [0, 3, 8] {
    let (reader, _, _) = readerOfASharedWindow { listing, _ in
      listing.inFront = [listed(40, process: sharedProcess, layer: layer, bounds: beside)]
    }
    let around = reader.read()
    check(around?.keyboard == .elsewhere, "layer \(layer): \(String(describing: around))")
    check(around?.fronts == [], "it lies beside the window, so it covers no point of it")
  }

  // A window of the application on a layer that holds no focus, or one that is not seen, or one of
  // another application, does not move it.
  for other in [
    listed(41, process: sharedProcess, layer: 9, bounds: beside),
    listed(42, process: sharedProcess, layer: -1, bounds: beside),
    listed(43, process: sharedProcess, layer: 0, alpha: 0, bounds: beside),
    listed(44, process: 300, layer: 0, bounds: beside),
    listed(45, process: sharedProcess, layer: 0, bounds: ScreenRect(x: 0, y: 0, width: 0, height: 0)),
    listed(46, process: sharedProcess, layer: 2000, bounds: beside),
  ] {
    let (reader, _, _) = readerOfASharedWindow { listing, _ in listing.inFront = [other] }
    check(reader.read()?.keyboard == .shared, "window \(other.id)")
  }
}

func testMoreWindowsInFrontThanTheRustSideTakesMakeNoAnswer() {
  let overlapping = (0..<4).map { listed(UInt32(50 + $0), process: 300) }
  let (three, _, _) = readerOfASharedWindow(mostFronts: 3) { listing, _ in
    listing.inFront = Array(overlapping.prefix(3))
  }
  check(three.read()?.fronts.count == 3)
  let (four, _, _) = readerOfASharedWindow(mostFronts: 3) { listing, _ in
    listing.inFront = overlapping
  }
  check(four.read() == nil)
  // The windows that lie elsewhere are not counted.
  let (beside, _, _) = readerOfASharedWindow(mostFronts: 3) { listing, _ in
    listing.inFront =
      overlapping + (0..<10).map { listed(UInt32(60 + $0), bounds: ScreenRect(x: 2000, y: 0, width: 5, height: 5)) }
  }
  check(beside.read() == nil, "four still overlap")
  check(SurroundingsReader.mostFronts == Int(BATON_FRONTS_MAX))
}

func testTheApplicationIsBroughtToTheFrontOnceAtTheFirstTouchBeforeTheWindowsInFrontAreRead() {
  var listingToChange: FakeListing?
  let (reader, listing, activity) = readerOfASharedWindow { listing, activity in
    activity.frontmostProcess = 300
    listingToChange = listing
    activity.onBring = { [unowned activity] in
      // The system makes the application the one in front, and its window comes up over the other.
      activity.frontmostProcess = sharedProcess
      listingToChange?.inFront = []
    }
    listing.inFront = [listed(70, process: 300)]
  }
  let first = reader.read()
  check(activity.brought == [sharedProcess], "\(activity.brought)")
  check(first?.keyboard == .shared && first?.fronts == [], "\(String(describing: first))")
  check(listing.describedIds.count == 1)

  // The person at the Mac brings another one forward: it is not fought for.
  activity.frontmostProcess = 300
  _ = reader.read()
  _ = reader.read()
  check(activity.brought == [sharedProcess], "\(activity.brought)")
}

func testTheFirstTouchWaitsForTheApplicationForWellLessThanAPictureMayBeOutOfDate() {
  // The thread of the input waits with it, and the rules judge what waits behind it by how long ago
  // its picture went out of date: 2 s in crates/baton-stream/src/gate.rs (`MAX_STALE`).
  let mostStale: TimeInterval = 2
  check(ActivationWait.total <= mostStale / 2, "\(ActivationWait.total) s")
  check(ActivationWait.total > 0)
  for wait in [
    ActivationWait.mainThread, ActivationWait.afterActivating, ActivationWait.opening,
    ActivationWait.afterOpening,
  ] {
    check(wait > 0, "a wait of \(wait) s")
  }
  check(
    ActivationWait.total
      == ActivationWait.mainThread + ActivationWait.afterActivating + ActivationWait.opening
      + ActivationWait.afterOpening)
}

func testTheApplicationIsNotBroughtForwardWhenItIsInFrontOrWhenTheMacDoesNotSay() {
  let (inFront, _, activity) = readerOfASharedWindow()
  _ = inFront.read()
  activity.frontmostProcess = 300
  _ = inFront.read()
  check(activity.brought.isEmpty, "the first touch found it in front, and that was the only try")

  let (silent, _, quiet) = readerOfASharedWindow { _, activity in activity.frontmostProcess = nil }
  _ = silent.read()
  quiet.frontmostProcess = 300
  _ = silent.read()
  check(quiet.brought.isEmpty)
}

func testAWindowThatIsNotOnTheScreenBringsNothingForwardAndTheFirstTouchAfterItIsBackDoes() {
  let (reader, listing, activity) = readerOfASharedWindow { listing, activity in
    activity.frontmostProcess = 300
    listing.described = .window(listed(sharedWindow, process: sharedProcess, onScreen: false))
  }
  // Minimized, on another desktop or its application hidden: bringing the application forward
  // would show it again, switch the desktop or reopen the window, and the session is paused.
  _ = reader.read()
  check(activity.brought.isEmpty, "a touch on a hidden window brought it: \(activity.brought)")

  listing.described = .gone
  _ = reader.read()
  check(activity.brought.isEmpty, "a touch on a closed window brought the application")

  let flat = ScreenRect(x: 0, y: 0, width: 0, height: 600)
  listing.described = .window(listed(sharedWindow, process: sharedProcess, bounds: flat))
  _ = reader.read()
  check(activity.brought.isEmpty, "a touch on a window with no size brought the application")

  listing.described = .unavailable
  _ = reader.read()
  check(activity.brought.isEmpty, "a window server that cannot be asked brought the application")

  // The window is on the screen again: the first touch after it brings the application, once.
  listing.described = .window(listed(sharedWindow, process: sharedProcess))
  _ = reader.read()
  check(activity.brought == [sharedProcess], "\(activity.brought)")
  _ = reader.read()
  check(activity.brought == [sharedProcess], "\(activity.brought)")
}

func testALockedMacIsNotTouchedAndTheFirstTouchAfterTheUnlockBringsTheApplication() {
  let (reader, _, activity) = readerOfASharedWindow { _, activity in
    activity.locked = true
    activity.frontmostProcess = 300
  }
  _ = reader.read()
  check(activity.brought.isEmpty)
  activity.locked = false
  _ = reader.read()
  check(activity.brought == [sharedProcess], "\(activity.brought)")
}

// MARK: - Pausing while the window cannot be seen

func testWhatTheWatchLooksAtIsTheLockAndWhetherTheWindowIsOnTheScreen() {
  let listing = FakeListing()
  let activity = FakeActivity()
  listing.described = .window(listed(sharedWindow))
  check(
    watchedState(window: sharedWindow, listing: listing, activity: activity)
      == WatchedState(locked: false, hidden: false))
  check(
    watchedState(window: nil, listing: listing, activity: activity)
      == WatchedState(locked: false, hidden: false), "a display has no number and is never hidden")

  listing.described = .window(listed(sharedWindow, onScreen: false))
  check(
    watchedState(window: sharedWindow, listing: listing, activity: activity)
      == WatchedState(locked: false, hidden: true))
  listing.described = .window(
    listed(sharedWindow, bounds: ScreenRect(x: 0, y: 0, width: 0, height: 0)))
  check(
    watchedState(window: sharedWindow, listing: listing, activity: activity)
      == WatchedState(locked: false, hidden: true))
  listing.described = .gone
  check(
    watchedState(window: sharedWindow, listing: listing, activity: activity)
      == WatchedState(locked: false, hidden: true))
  listing.described = .unavailable
  check(watchedState(window: sharedWindow, listing: listing, activity: activity) == nil)
  check(
    watchedState(window: nil, listing: listing, activity: activity)
      == WatchedState(locked: false, hidden: false), "a display is not asked about")

  // A locked Mac is not asked about its windows, and a display is locked with it.
  activity.locked = true
  listing.described = .window(listed(sharedWindow))
  let before = listing.describedIds.count
  check(
    watchedState(window: sharedWindow, listing: listing, activity: activity)
      == WatchedState(locked: true, hidden: false))
  check(
    watchedState(window: nil, listing: listing, activity: activity)
      == WatchedState(locked: true, hidden: false))
  check(listing.describedIds.count == before)
}

func testThePauseWatchTellsTheLibraryOnlyWhenTheAnswerChanges() {
  let session = FakeSession()
  var states: [WatchedState?] = []
  let watch = PauseWatch(session: session) { states.isEmpty ? nil : states.removeFirst() }
  let visible = WatchedState(locked: false, hidden: false)

  states = [visible, visible]
  watch.poll()
  watch.poll()
  check(session.calls.isEmpty, "a window that is seen is not resumed: \(session.calls)")

  states = [WatchedState(locked: false, hidden: true), WatchedState(locked: false, hidden: true)]
  watch.poll()
  watch.poll()
  check(session.pauses == [.hidden] && watch.paused == .hidden, "\(session.pauses)")

  // A Mac that is locked while the window is hidden is locked first, and the reason is told again.
  states = [WatchedState(locked: true, hidden: true), WatchedState(locked: true, hidden: false)]
  watch.poll()
  check(session.pauses == [.hidden, .locked] && watch.paused == .locked, "\(session.pauses)")
  watch.poll()
  check(session.pauses == [.hidden, .locked] && watch.paused == .locked, "\(session.pauses)")

  // A question that cannot be answered changes nothing.
  states = [nil]
  watch.poll()
  check(session.pauses == [.hidden, .locked] && session.resumes == 0)

  states = [WatchedState(locked: false, hidden: true)]
  watch.poll()
  check(session.pauses == [.hidden, .locked, .hidden], "\(session.pauses)")

  states = [visible, visible]
  watch.poll()
  watch.poll()
  check(session.resumes == 1 && watch.paused == nil, "\(session.resumes)")
  check(session.calls == ["pause", "pause", "pause", "resume"], "\(session.calls)")
}

// MARK: - The geometry of a picture

func testTheGeometryOfAPictureHoldsTheWindowAndWhereItIsInThePicture() {
  // A window of 800 by 600 points drawn at twice the size, with a margin above and below it.
  let window = ScreenRect(x: 40, y: 50, width: 800, height: 600)
  let content = ScreenRect(x: 0, y: 150, width: 1600, height: 1200)
  check(
    pictureGeometry(window: window, content: content, pictureWidth: 1600, pictureHeight: 1500)
      == PictureGeometry(
        windowWidth: 800, windowHeight: 600, pictureWidth: 1600, pictureHeight: 1500,
        content: content))
}

func testARectangleThatIsNotTheWindowScaledToFitIsNotItsGeometry() {
  let window = ScreenRect(x: 0, y: 0, width: 800, height: 600)
  func geometry(_ content: ScreenRect, _ width: UInt32, _ height: UInt32) -> PictureGeometry? {
    pictureGeometry(window: window, content: content, pictureWidth: width, pictureHeight: height)
  }
  // The size in points where pixels are meant: the window would be taken for a quarter of the
  // picture, and a tap would land at half of where the person pointed.
  check(geometry(ScreenRect(x: 0, y: 0, width: 800, height: 600), 1600, 1200) == nil)
  // Not the shape of the window.
  check(geometry(ScreenRect(x: 0, y: 0, width: 1600, height: 900), 1600, 1200) == nil)
  check(geometry(ScreenRect(x: 0, y: 0, width: 1440, height: 1200), 1600, 1200) == nil)
  // Smaller than the scale to fit makes it, on the axis that is not the one that fills.
  check(geometry(ScreenRect(x: 0, y: 150, width: 1600, height: 1196), 1600, 1500) == nil)
  // Larger than the picture allows it to be, and the right shape for a larger one.
  check(geometry(ScreenRect(x: 0, y: 0, width: 800, height: 600), 800, 600) != nil)
  check(geometry(ScreenRect(x: 0, y: 0, width: 1600, height: 1200), 800, 600) == nil)
  // The window drawn smaller than the picture on both axes is not a scale to fit.
  check(geometry(ScreenRect(x: 400, y: 300, width: 800, height: 600), 1600, 1200) == nil)
  // Two pixels of rounding on either size are allowed, and not three.
  check(geometry(ScreenRect(x: 0, y: 150, width: 1600, height: 1202), 1600, 1500) != nil)
  check(geometry(ScreenRect(x: 0, y: 150, width: 1600, height: 1198), 1600, 1500) != nil)
  check(geometry(ScreenRect(x: 0, y: 150, width: 1600, height: 1203), 1600, 1500) == nil)
  check(geometry(ScreenRect(x: 0, y: 150, width: 1600, height: 1197), 1600, 1500) == nil)
  // A picture that is narrower than the shape of the window leaves its margin at the sides.
  check(geometry(ScreenRect(x: 100, y: 0, width: 1200, height: 900), 1400, 900) != nil)
}

func testAGeometryThatDoesNotFitThePictureIsNone() {
  let window = ScreenRect(x: 0, y: 0, width: 800, height: 600)
  let content = ScreenRect(x: 0, y: 0, width: 1600, height: 1200)
  func geometry(
    _ window: ScreenRect?, _ content: ScreenRect?, _ width: UInt32 = 1600, _ height: UInt32 = 1200
  ) -> PictureGeometry? {
    pictureGeometry(window: window, content: content, pictureWidth: width, pictureHeight: height)
  }
  check(geometry(window, content) != nil)
  check(geometry(nil, content) == nil)
  check(geometry(window, nil) == nil)
  check(geometry(ScreenRect(x: 0, y: 0, width: 0, height: 600), content) == nil)
  check(geometry(ScreenRect(x: 0, y: 0, width: 800, height: .nan), content) == nil)
  check(geometry(window, ScreenRect(x: 0, y: 0, width: 1600, height: 0)) == nil)
  check(geometry(window, content, 0, 1200) == nil)
  check(geometry(window, content, 1600, 0) == nil)
  // A window somewhere else than in this picture.
  check(geometry(window, ScreenRect(x: 0, y: 0, width: 1602, height: 1200)) == nil)
  check(geometry(window, ScreenRect(x: 0, y: 0, width: 1600, height: 1202)) == nil)
  check(geometry(window, ScreenRect(x: -2, y: 0, width: 1600, height: 1200)) == nil)
  check(geometry(window, ScreenRect(x: 0, y: -2, width: 1600, height: 1200)) == nil)
  check(geometry(window, ScreenRect(x: 1700, y: 0, width: 10, height: 10)) == nil)
  // A window with no size, or a size below zero, has no scale, even where the numbers would fit:
  // minus 800 by minus 600 at a scale of minus 2 is the picture.
  check(geometry(ScreenRect(x: 0, y: 0, width: -800, height: -600), content) == nil)
  check(
    geometry(
      ScreenRect(x: 0, y: 0, width: 0, height: 600), ScreenRect(x: 0, y: 0, width: 1, height: 1200))
      == nil)
  check(
    geometry(
      ScreenRect(x: 0, y: 0, width: 800, height: 0), ScreenRect(x: 0, y: 0, width: 1600, height: 1))
      == nil)
  // A pixel of rounding is allowed, to the pixel and on every side, and the rectangle is put
  // inside the picture.
  for (said, kept) in [
    (ScreenRect(x: -1, y: 0, width: 1600, height: 1200), ScreenRect(x: 0, y: 0, width: 1599, height: 1200)),
    (ScreenRect(x: 0, y: -1, width: 1600, height: 1200), ScreenRect(x: 0, y: 0, width: 1600, height: 1199)),
    (ScreenRect(x: 1, y: 0, width: 1600, height: 1200), ScreenRect(x: 1, y: 0, width: 1599, height: 1200)),
    (ScreenRect(x: 0, y: 1, width: 1600, height: 1200), ScreenRect(x: 0, y: 1, width: 1600, height: 1199)),
  ] {
    check(geometry(window, said)?.content == kept, "\(said): \(String(describing: geometry(window, said)))")
  }
  check(geometry(window, ScreenRect(x: -1.001, y: 0, width: 1600, height: 1200)) == nil)
  check(geometry(window, ScreenRect(x: 0, y: -1.001, width: 1600, height: 1200)) == nil)
  check(geometry(window, ScreenRect(x: 1.001, y: 0, width: 1600, height: 1200)) == nil)
  check(geometry(window, ScreenRect(x: 0, y: 1.001, width: 1600, height: 1200)) == nil)
  // The size of the window in the picture is allowed two pixels of rounding on the width as well.
  check(geometry(window, ScreenRect(x: 0, y: 0, width: 1598, height: 1200)) != nil)
  check(geometry(window, ScreenRect(x: 0, y: 0, width: 1597.9, height: 1200)) == nil)
  check(geometry(window, ScreenRect(x: -1, y: 0, width: 1602, height: 1200)) != nil)
  // A window that is outside the picture but for the pixel of rounding has nothing in it.
  let pixel = ScreenRect(x: 0, y: 0, width: 1, height: 1)
  func inAPixel(_ content: ScreenRect) -> PictureGeometry? {
    pictureGeometry(window: pixel, content: content, pictureWidth: 1, pictureHeight: 1)
  }
  check(inAPixel(ScreenRect(x: 0, y: 0, width: 1, height: 1)) != nil)
  check(inAPixel(ScreenRect(x: -1, y: -1, width: 1, height: 1)) == nil, "left of and above it")
  check(inAPixel(ScreenRect(x: 1, y: 1, width: 1, height: 1)) == nil, "right of and below it")
  check(
    geometry(window, ScreenRect(x: -0.5, y: -1, width: 1601, height: 1201))?.content
      == ScreenRect(x: 0, y: 0, width: 1600, height: 1200))
  check(
    geometry(window, ScreenRect(x: 0.25, y: 0.5, width: 1600.5, height: 1200))?.content
      == ScreenRect(x: 0.25, y: 0.5, width: 1599.75, height: 1199.5))
}

func testARectangleIsReadFromTheDictionaryThatTheSystemGives() {
  func read(_ value: Any?) -> ScreenRect? { screenRect(fromDictionary: value) }
  let numbers: [String: Any] = ["X": NSNumber(value: 10), "Y": NSNumber(value: 20.5),
    "Width": NSNumber(value: 800), "Height": NSNumber(value: 600)]
  check(read(numbers) == ScreenRect(x: 10, y: 20.5, width: 800, height: 600))
  // The numbers may be negative: a window on a display to the left of the main one.
  check(
    read(["X": -1440.0, "Y": -200, "Width": 1440, "Height": 900] as [String: Any])
      == ScreenRect(x: -1440, y: -200, width: 1440, height: 900))
  // Something missing, something that is not a number, and something that is not a dictionary.
  for key in ["X", "Y", "Width", "Height"] {
    var without = numbers
    without[key] = nil
    check(read(without) == nil, "no \(key)")
    var text = numbers
    text[key] = "4"
    check(read(text) == nil, "\(key) is text")
  }
  check(read(["x": 1, "y": 2, "width": 3, "height": 4] as [String: Any]) == nil)
  check(read([1, 2, 3, 4]) == nil)
  check(read(5) == nil)
  check(read(nil) == nil)
}

// MARK: - The layout of the images of a capture

final class LayoutNotes {
  var told: [String] = []
}

/// A layout for the window that is 800 by 600 points on the screen, whose server is `listing`.
func layoutOfASharedWindow(
  _ clock: TestClock = TestClock(), _ notes: LayoutNotes = LayoutNotes()
) -> (CaptureLayout, FakeListing) {
  let listing = FakeListing()
  listing.described = .window(listed(sharedWindow, process: sharedProcess))
  let layout = CaptureLayout(
    window: sharedWindow, listing: listing, now: { clock.seconds }, note: { notes.told.append($0) })
  return (layout, listing)
}

func testTheLayoutOfAnImageIsTheWindowAndWhatTheCaptureSaysOfIt() {
  let (layout, _) = layoutOfASharedWindow()
  // The window is 800 by 600 points, drawn at twice the size in a picture with a margin above and
  // below.
  let said = ScreenRect(x: 0, y: 150, width: 1600, height: 1200)
  check(
    layout.geometry(pictureWidth: 1600, pictureHeight: 1500, reported: said)
      == PictureGeometry(
        windowWidth: 800, windowHeight: 600, pictureWidth: 1600, pictureHeight: 1500, content: said))
}

func testWhenTheCaptureSaysNothingThatFitsAWindowOfTheShapeOfThePictureFillsIt() {
  let whole = PictureGeometry(
    windowWidth: 800, windowHeight: 600, pictureWidth: 1600, pictureHeight: 1200,
    content: ScreenRect(x: 0, y: 0, width: 1600, height: 1200))
  // Nothing said.
  var (layout, _) = layoutOfASharedWindow()
  check(layout.geometry(pictureWidth: 1600, pictureHeight: 1200, reported: nil) == whole)
  // Said in points where pixels are meant.
  (layout, _) = layoutOfASharedWindow()
  check(
    layout.geometry(
      pictureWidth: 1600, pictureHeight: 1200,
      reported: ScreenRect(x: 0, y: 0, width: 800, height: 600)) == whole)
  // Said somewhere else than in the picture.
  (layout, _) = layoutOfASharedWindow()
  check(
    layout.geometry(
      pictureWidth: 1600, pictureHeight: 1200,
      reported: ScreenRect(x: 900, y: 0, width: 1600, height: 1200)) == whole)
}

func testWhenNothingFitsTheImageHasNoLayoutAndTheNumbersAreTold() {
  let notes = LayoutNotes()
  let (layout, _) = layoutOfASharedWindow(TestClock(), notes)
  // A picture of the shape of the window with a margin at the sides, and nothing said of it: it is
  // not known where the window is.
  check(layout.geometry(pictureWidth: 1800, pictureHeight: 1200, reported: nil) == nil)
  check(notes.told.count == 1, "\(notes.told)")
  let told = notes.told.first ?? ""
  check(told.contains("none"), told)
  check(told.contains("100.0,100.0 800.0x600.0"), told)
  check(told.contains("1800x1200"), told)
  check(told.contains("capture says nothing"), told)
}

func testTheLayoutIsToldWhenItChangesAndOnlyThen() {
  let clock = TestClock()
  let notes = LayoutNotes()
  let (layout, _) = layoutOfASharedWindow(clock, notes)
  let said = ScreenRect(x: 0, y: 0, width: 1600, height: 1200)
  for _ in 0..<5 {
    check(layout.geometry(pictureWidth: 1600, pictureHeight: 1200, reported: said) != nil)
  }
  check(notes.told.count == 1 && notes.told[0].contains("reported"), "\(notes.told)")
  // It is the whole picture now, and then none, and then what the capture says again.
  check(layout.geometry(pictureWidth: 1600, pictureHeight: 1200, reported: nil) != nil)
  check(layout.geometry(pictureWidth: 1600, pictureHeight: 1200, reported: nil) != nil)
  check(notes.told.count == 2 && notes.told[1].contains("wholePicture"), "\(notes.told)")
  check(layout.geometry(pictureWidth: 1800, pictureHeight: 1200, reported: nil) == nil)
  check(layout.geometry(pictureWidth: 1800, pictureHeight: 1200, reported: nil) == nil)
  check(notes.told.count == 3 && notes.told[2].contains("none"), "\(notes.told)")
  check(layout.geometry(pictureWidth: 1600, pictureHeight: 1200, reported: said) != nil)
  check(notes.told.count == 4 && notes.told[3].contains("reported"), "\(notes.told)")
}

func testAWindowThatTheWindowServerDoesNotHaveHasNoLayout() {
  let said = ScreenRect(x: 0, y: 0, width: 1600, height: 1200)
  for described in [DescribedWindow.gone, .unavailable] {
    let notes = LayoutNotes()
    let clock = TestClock()
    let (layout, listing) = layoutOfASharedWindow(clock, notes)
    listing.described = described
    check(layout.geometry(pictureWidth: 1600, pictureHeight: 1200, reported: said) == nil)
    check(notes.told.count == 1 && notes.told[0].contains("window nothing"), "\(notes.told)")
    // It comes back, and the layout with it.
    clock.seconds += 1
    listing.described = .window(listed(sharedWindow, process: sharedProcess))
    check(layout.geometry(pictureWidth: 1600, pictureHeight: 1200, reported: said) != nil)
  }
}

func testAReadingOfTheWindowServerIsUsedForLessThanItsTimeAndNotForAllOfIt() {
  // The clock starts at zero, so that the sums of the seconds are exact.
  let clock = TestClock()
  clock.seconds = 0
  let (layout, listing) = layoutOfASharedWindow(clock)
  let said = ScreenRect(x: 0, y: 0, width: 1600, height: 1200)
  func image() { _ = layout.geometry(pictureWidth: 1600, pictureHeight: 1200, reported: said) }
  image()
  check(listing.describedIds.count == 1)
  clock.seconds = CaptureLayout.reuse.nextDown
  image()
  check(listing.describedIds.count == 1, "just before its time: \(listing.describedIds)")
  clock.seconds = CaptureLayout.reuse
  image()
  check(listing.describedIds.count == 2, "at its time: \(listing.describedIds)")
}

func testTheWindowServerIsAskedOnceForManyImages() {
  let clock = TestClock()
  let (layout, listing) = layoutOfASharedWindow(clock)
  let said = ScreenRect(x: 0, y: 0, width: 1600, height: 1200)
  for _ in 0..<10 {
    _ = layout.geometry(pictureWidth: 1600, pictureHeight: 1200, reported: said)
    clock.seconds += 0.004
  }
  check(listing.describedIds == [sharedWindow], "\(listing.describedIds)")
  // After the time that a reading is used for, the window is asked about again, and a window that
  // changed its size in the meantime changes the geometry.
  listing.described = .window(
    listed(sharedWindow, process: sharedProcess, bounds: ScreenRect(x: 0, y: 0, width: 400, height: 300)))
  clock.seconds += CaptureLayout.reuse
  let now = layout.geometry(
    pictureWidth: 1600, pictureHeight: 1200,
    reported: ScreenRect(x: 0, y: 0, width: 1600, height: 1200))
  check(listing.describedIds.count == 2, "\(listing.describedIds)")
  check(now?.windowWidth == 400 && now?.windowHeight == 300, "\(String(describing: now))")
  // A clock that goes back is not trusted.
  clock.seconds -= 10
  _ = layout.geometry(pictureWidth: 1600, pictureHeight: 1200, reported: nil)
  check(listing.describedIds.count == 3, "\(listing.describedIds)")
}

/// Where the system says a display is, and how often it was asked.
final class DisplayPlace {
  var bounds: ScreenRect? = displayBounds
  private(set) var asked = 0

  func ask() -> ScreenRect? {
    asked += 1
    return bounds
  }
}

/// A layout for the display that is 1512 by 982 points on the screen.
func layoutOfASharedDisplay(
  _ clock: TestClock = TestClock(), _ notes: LayoutNotes = LayoutNotes()
) -> (CaptureLayout, DisplayPlace) {
  let place = DisplayPlace()
  let layout = CaptureLayout(
    noun: "screen", bounds: { place.ask() }, now: { clock.seconds },
    note: { notes.told.append($0) })
  return (layout, place)
}

func testTheLayoutOfAnImageOfADisplayIsTheDisplayAndWhatTheCaptureSaysOfIt() {
  let (layout, _) = layoutOfASharedDisplay()
  // The display is 1512 by 982 points, drawn at twice the size, as the pixels of its screen are.
  let said = ScreenRect(x: 0, y: 0, width: 3024, height: 1964)
  check(
    layout.geometry(pictureWidth: 3024, pictureHeight: 1964, reported: said)
      == PictureGeometry(
        windowWidth: 1512, windowHeight: 982, pictureWidth: 3024, pictureHeight: 1964,
        content: said))
}

func testWhenTheCaptureSaysNothingADisplayOfTheShapeOfThePictureFillsIt() {
  let (layout, _) = layoutOfASharedDisplay()
  // Scaled down to the picture that the encoder takes, which has the shape of the display to within
  // a pixel.
  check(
    layout.geometry(pictureWidth: 1920, pictureHeight: 1247, reported: nil)
      == PictureGeometry(
        windowWidth: 1512, windowHeight: 982, pictureWidth: 1920, pictureHeight: 1247,
        content: ScreenRect(x: 0, y: 0, width: 1920, height: 1247)))
  // A picture of another shape has no layout.
  check(layout.geometry(pictureWidth: 1920, pictureHeight: 1080, reported: nil) == nil)
}

func testTheLayoutOfADisplayIsToldAsAScreenWithTheNumbersAndNothingMore() {
  let notes = LayoutNotes()
  let (layout, _) = layoutOfASharedDisplay(TestClock(), notes)
  _ = layout.geometry(pictureWidth: 3024, pictureHeight: 1964, reported: nil)
  check(notes.told.count == 1, "\(notes.told)")
  let told = notes.told.first ?? ""
  check(told.contains("wholePicture"), told)
  check(told.contains("screen 0.0,0.0 1512.0x982.0 points"), told)
  check(told.contains("3024x1964"), told)
  check(!told.contains("window"), told)
}

func testADisplayThatTheSystemDoesNotHaveHasNoLayoutUntilItComesBack() {
  let clock = TestClock()
  let notes = LayoutNotes()
  let (layout, place) = layoutOfASharedDisplay(clock, notes)
  let said = ScreenRect(x: 0, y: 0, width: 3024, height: 1964)
  place.bounds = nil
  check(layout.geometry(pictureWidth: 3024, pictureHeight: 1964, reported: said) == nil)
  check(notes.told.count == 1 && notes.told[0].contains("screen nothing"), "\(notes.told)")
  clock.seconds += 1
  place.bounds = displayBounds
  check(layout.geometry(pictureWidth: 3024, pictureHeight: 1964, reported: said) != nil)
}

func testTheSystemIsAskedOnceForManyImagesOfADisplay() {
  let clock = TestClock()
  let (layout, place) = layoutOfASharedDisplay(clock)
  for _ in 0..<10 {
    _ = layout.geometry(pictureWidth: 3024, pictureHeight: 1964, reported: nil)
    clock.seconds += 0.004
  }
  check(place.asked == 1, "\(place.asked)")
  clock.seconds += CaptureLayout.reuse
  _ = layout.geometry(pictureWidth: 3024, pictureHeight: 1964, reported: nil)
  check(place.asked == 2, "\(place.asked)")
}

// MARK: - The keys of a Mac

/// The keys of a US keyboard with the virtual key codes that HIToolbox/Events.h gives them
/// (kVK_ANSI_), typed again from the header, and the usage of each in the HID Usage Tables.
let ansiKeys: [(character: Character, usage: UInt32, virtualKey: UInt16)] = [
  ("a", 0x04, 0x00), ("b", 0x05, 0x0B), ("c", 0x06, 0x08), ("d", 0x07, 0x02),
  ("e", 0x08, 0x0E), ("f", 0x09, 0x03), ("g", 0x0A, 0x05), ("h", 0x0B, 0x04),
  ("i", 0x0C, 0x22), ("j", 0x0D, 0x26), ("k", 0x0E, 0x28), ("l", 0x0F, 0x25),
  ("m", 0x10, 0x2E), ("n", 0x11, 0x2D), ("o", 0x12, 0x1F), ("p", 0x13, 0x23),
  ("q", 0x14, 0x0C), ("r", 0x15, 0x0F), ("s", 0x16, 0x01), ("t", 0x17, 0x11),
  ("u", 0x18, 0x20), ("v", 0x19, 0x09), ("w", 0x1A, 0x0D), ("x", 0x1B, 0x07),
  ("y", 0x1C, 0x10), ("z", 0x1D, 0x06),
  ("1", 0x1E, 0x12), ("2", 0x1F, 0x13), ("3", 0x20, 0x14), ("4", 0x21, 0x15),
  ("5", 0x22, 0x17), ("6", 0x23, 0x16), ("7", 0x24, 0x1A), ("8", 0x25, 0x1C),
  ("9", 0x26, 0x19), ("0", 0x27, 0x1D),
  ("-", 0x2D, 0x1B), ("=", 0x2E, 0x18), ("[", 0x2F, 0x21), ("]", 0x30, 0x1E),
  ("\\", 0x31, 0x2A), (";", 0x33, 0x29), ("'", 0x34, 0x27), ("`", 0x35, 0x32),
  (",", 0x36, 0x2B), (".", 0x37, 0x2F), ("/", 0x38, 0x2C),
]

/// The keys that are not characters, by name.
let namedKeys: [(name: String, usage: UInt32, virtualKey: UInt16)] = [
  ("return", 0x28, 0x24), ("escape", 0x29, 0x35), ("backspace", 0x2A, 0x33), ("tab", 0x2B, 0x30),
  ("space", 0x2C, 0x31), ("caps lock", 0x39, 0x39),
  ("f1", 0x3A, 0x7A), ("f2", 0x3B, 0x78), ("f3", 0x3C, 0x63), ("f4", 0x3D, 0x76),
  ("f5", 0x3E, 0x60), ("f6", 0x3F, 0x61), ("f7", 0x40, 0x62), ("f8", 0x41, 0x64),
  ("f9", 0x42, 0x65), ("f10", 0x43, 0x6D), ("f11", 0x44, 0x67), ("f12", 0x45, 0x6F),
  ("insert", 0x49, 0x72), ("home", 0x4A, 0x73), ("page up", 0x4B, 0x74),
  ("delete forward", 0x4C, 0x75), ("end", 0x4D, 0x77), ("page down", 0x4E, 0x79),
  ("right", 0x4F, 0x7C), ("left", 0x50, 0x7B), ("down", 0x51, 0x7D), ("up", 0x52, 0x7E),
  ("num lock", 0x53, 0x47), ("keypad /", 0x54, 0x4B), ("keypad *", 0x55, 0x43),
  ("keypad -", 0x56, 0x4E), ("keypad +", 0x57, 0x45), ("keypad enter", 0x58, 0x4C),
  ("keypad 1", 0x59, 0x53), ("keypad 2", 0x5A, 0x54), ("keypad 3", 0x5B, 0x55),
  ("keypad 4", 0x5C, 0x56), ("keypad 5", 0x5D, 0x57), ("keypad 6", 0x5E, 0x58),
  ("keypad 7", 0x5F, 0x59), ("keypad 8", 0x60, 0x5B), ("keypad 9", 0x61, 0x5C),
  ("keypad 0", 0x62, 0x52), ("keypad .", 0x63, 0x41), ("iso section", 0x64, 0x0A),
  ("menu", 0x65, 0x6E), ("keypad =", 0x67, 0x51),
  ("f13", 0x68, 0x69), ("f14", 0x69, 0x6B), ("f15", 0x6A, 0x71), ("f16", 0x6B, 0x6A),
  ("f17", 0x6C, 0x40), ("f18", 0x6D, 0x4F), ("f19", 0x6E, 0x50), ("f20", 0x6F, 0x5A),
]

func testEveryKeyOfAUsKeyboardHasTheCodeOfItsPositionOnAMac() {
  for key in ansiKeys {
    check(
      macKey(forUsage: key.usage) == key.virtualKey,
      "\(key.character): \(String(describing: macKey(forUsage: key.usage)))")
  }
  for key in namedKeys {
    check(
      macKey(forUsage: key.usage) == key.virtualKey,
      "\(key.name): \(String(describing: macKey(forUsage: key.usage)))")
  }
}

func testTheTableOfKeysHasNoOtherKeysAndNoTwoUsagesForOneCode() {
  let known = Set(ansiKeys.map { $0.usage } + namedKeys.map { $0.usage })
  check(Set(macKeysByUsage.keys) == known, "\(Set(macKeysByUsage.keys).symmetricDifference(known))")
  check(macKeysByUsage.count == ansiKeys.count + namedKeys.count, "\(macKeysByUsage.count)")
  check(
    Set(macKeysByUsage.values).count == macKeysByUsage.count,
    "two usages have the same code")
  // A usage that is not a key of the page: the roll-over codes, the keys that are not here, and
  // everything beyond the last modifier.
  for usage: UInt32 in [0, 1, 2, 3, 0x32, 0x46, 0x47, 0x48, 0x66, 0x70, 0xDF, 0xE8, 0x1_0007] {
    check(macKey(forUsage: usage) == nil, "usage \(usage)")
  }
}

func testTheModifierKeysAreTheEightUsagesAtTheEndOfThePage() {
  for usage: UInt32 in 0xE0...0xE7 { check(isModifierUsage(usage), "\(usage)") }
  for usage: UInt32 in [0, 0x04, 0xDF, 0xE8, 0xFFFF] { check(!isModifierUsage(usage), "\(usage)") }
  check(macKeysByUsage.keys.allSatisfy { !isModifierUsage($0) })
}

func testTheModifiersAreTheBitsOfTheInterface() {
  check(InputModifiers.shift.rawValue == BATON_MOD_SHIFT)
  check(InputModifiers.control.rawValue == BATON_MOD_CONTROL)
  check(InputModifiers.option.rawValue == BATON_MOD_OPTION)
  check(InputModifiers.command.rawValue == BATON_MOD_COMMAND)
  check(InputModifiers.capsLock.rawValue == BATON_MOD_CAPS_LOCK)
}

// MARK: - Turning a key into events

func key(
  _ code: UInt32, down: Bool = true, repeating: Bool = false, modifiers: InputModifiers = [],
  text: String? = nil
) -> InputPost {
  .key(KeyPost(code: code, down: down, repeating: repeating, modifiers: modifiers, text: text))
}

func syntheticKey(
  _ virtualKey: UInt16, down: Bool = true, autorepeat: Bool = false,
  modifiers: InputModifiers = [], text: [UInt16]? = nil
) -> SyntheticEvent {
  .key(
    SyntheticKey(
      virtualKey: virtualKey, down: down, autorepeat: autorepeat, modifiers: modifiers, text: text))
}

func testAKeyGoesDownAndComesUpWithTheModifiersThatAreDown() {
  let translator = InputTranslator()
  let shift: InputModifiers = [.shift, .command]
  check(
    translator.events(for: key(0x04, modifiers: shift)) == [syntheticKey(0x00, modifiers: shift)])
  check(
    translator.events(for: key(0x04, down: false, modifiers: shift))
      == [syntheticKey(0x00, down: false, modifiers: shift)])
  check(translator.events(for: key(0x52)) == [syntheticKey(0x7E)])
}

func testAKeyThatIsHeldRepeatsOnlyWhileItGoesDown() {
  let translator = InputTranslator()
  check(
    translator.events(for: key(0x2C, repeating: true)) == [syntheticKey(0x31, autorepeat: true)])
  check(translator.events(for: key(0x2C, repeating: false)) == [syntheticKey(0x31)])
  check(
    translator.events(for: key(0x2C, down: false, repeating: true))
      == [syntheticKey(0x31, down: false)], "a release is not a repeat")
}

func testAModifierKeyMakesNoEventAndAKeyWithNoCodeOnAMacMakesNone() {
  let translator = InputTranslator()
  for usage: UInt32 in 0xE0...0xE7 {
    check(translator.events(for: key(usage)) == [], "usage \(usage) down")
    check(translator.events(for: key(usage, down: false)) == [], "usage \(usage) up")
  }
  for usage: UInt32 in [3, 0x32, 0x66, 0xE8, 0x1_0007] {
    check(translator.events(for: key(usage)) == nil, "usage \(usage)")
  }
}

func testTheTextOfAKeyIsLeftToTheKeyAndTextWithNoKeyIsTypedOnce() {
  let translator = InputTranslator()
  check(
    translator.events(for: key(0x04, text: "a"))
      == [syntheticKey(0x00)], "the key types what the layout of the Mac says")

  let units = Array("Käse 🧀".utf16)
  check(
    translator.events(for: key(0, text: "Käse 🧀"))
      == [syntheticKey(0, text: units), syntheticKey(0, down: false, text: units)])
  check(
    translator.events(for: key(0, modifiers: [.shift], text: "A"))
      == [
        syntheticKey(0, modifiers: [.shift], text: [65]),
        syntheticKey(0, down: false, modifiers: [.shift], text: [65]),
      ])
  // Text is not held or repeated.
  check(
    translator.events(for: key(0, repeating: true, text: "b"))
      == [syntheticKey(0, text: [98]), syntheticKey(0, down: false, text: [98])])
  check(translator.events(for: key(0, down: false, text: "b")) == [])
  check(translator.events(for: key(0)) == nil, "text with no text")
  check(translator.events(for: key(0, text: "")) == nil)
}

func testTextOfMoreThanTwentyUnitsOfUtf16IsNotTyped() {
  let translator = InputTranslator()
  let twenty = String(repeating: "x", count: 20)
  check(translator.events(for: key(0, text: twenty))?.count == 2)
  check(translator.events(for: key(0, text: twenty + "x")) == nil)
  // An emoji is two units.
  check(translator.events(for: key(0, text: String(repeating: "🧀", count: 10)))?.count == 2)
  check(translator.events(for: key(0, text: String(repeating: "🧀", count: 11))) == nil)
  check(InputTranslator.mostTextUnits == 20)
}

// MARK: - Turning a pointer into events

func pointer(
  _ action: PointerAction, at: (Double, Double) = (300, 200), button: PointerButton = .left,
  dx: Double = 0, dy: Double = 0, modifiers: InputModifiers = []
) -> InputPost {
  .pointer(
    PointerPost(
      at: ScreenPoint(x: at.0, y: at.1), action: action, button: button, dx: dx, dy: dy,
      modifiers: modifiers))
}

func mouse(
  _ kind: MouseKind, at: (Double, Double) = (300, 200), button: PointerButton = .left,
  count: Int, pressure: Double, modifiers: InputModifiers = []
) -> SyntheticEvent {
  .mouse(
    SyntheticMouse(
      kind: kind, button: button, at: ScreenPoint(x: at.0, y: at.1), clickCount: count,
      pressure: pressure, modifiers: modifiers))
}

/// A clock that a test sets.
final class TestClock {
  var seconds = 100.0
}

func translatorWithAClock() -> (InputTranslator, TestClock) {
  let clock = TestClock()
  let translator = InputTranslator(
    doubleClickInterval: 0.5, clickSlop: 4, now: { clock.seconds })
  return (translator, clock)
}

func testAPointerMovesPressesDragsAndReleases() {
  let (translator, _) = translatorWithAClock()
  check(
    translator.events(for: pointer(.move, at: (10, 20)))
      == [mouse(.moved, at: (10, 20), count: 0, pressure: 0)])
  check(
    translator.events(for: pointer(.down, at: (11, 21), modifiers: [.shift]))
      == [mouse(.down, at: (11, 21), count: 1, pressure: 1, modifiers: [.shift])])
  check(
    translator.events(for: pointer(.move, at: (15, 25), modifiers: [.shift]))
      == [mouse(.dragged, at: (15, 25), count: 1, pressure: 1, modifiers: [.shift])])
  check(
    translator.events(for: pointer(.up, at: (16, 26)))
      == [mouse(.up, at: (16, 26), count: 1, pressure: 0)])
  check(
    translator.events(for: pointer(.move, at: (30, 40)))
      == [mouse(.moved, at: (30, 40), count: 0, pressure: 0)], "a button that is up is not held")
}

func testTheButtonsAreTheirOwn() {
  let (translator, clock) = translatorWithAClock()
  for button in [PointerButton.right, .middle, .left] {
    check(
      translator.events(for: pointer(.down, button: button))
        == [mouse(.down, button: button, count: 1, pressure: 1)], "\(button)")
    // A move while a button is down is a drag of that button.
    check(
      translator.events(for: pointer(.move, button: .left))
        == [mouse(.dragged, button: button, count: 1, pressure: 1)], "drag of \(button)")
    check(
      translator.events(for: pointer(.up, button: button))
        == [mouse(.up, button: button, count: 1, pressure: 0)], "\(button)")
    clock.seconds += 10
  }
}

func testWithTwoButtonsDownTheFirstOneDragsAndAReleaseLetsGoOfOnlyItsOwnButton() {
  let (translator, _) = translatorWithAClock()
  _ = translator.events(for: pointer(.down, button: .left))
  _ = translator.events(for: pointer(.down, button: .right))
  // A move is a drag of the button that went down first, whichever button the move names.
  check(
    translator.events(for: pointer(.move, button: .right))
      == [mouse(.dragged, button: .left, count: 1, pressure: 1)])
  // The one that is let go of leaves the other down, which drags now.
  check(
    translator.events(for: pointer(.up, button: .left))
      == [mouse(.up, button: .left, count: 1, pressure: 0)])
  check(
    translator.events(for: pointer(.move))
      == [mouse(.dragged, button: .right, count: 1, pressure: 1)])
  check(
    translator.events(for: pointer(.up, button: .right))
      == [mouse(.up, button: .right, count: 1, pressure: 0)])
  check(translator.events(for: pointer(.move)) == [mouse(.moved, count: 0, pressure: 0)])

  // A button that goes down again while it is still down is down once, and went down last.
  let (again, _) = translatorWithAClock()
  _ = again.events(for: pointer(.down, button: .left))
  _ = again.events(for: pointer(.down, button: .right))
  _ = again.events(for: pointer(.down, button: .left))
  check(
    again.events(for: pointer(.move))
      == [mouse(.dragged, button: .right, count: 1, pressure: 1)])
  _ = again.events(for: pointer(.up, button: .right))
  check(
    again.events(for: pointer(.move))
      == [mouse(.dragged, button: .left, count: 1, pressure: 1)])
  _ = again.events(for: pointer(.up, button: .left))
  check(again.events(for: pointer(.move)) == [mouse(.moved, count: 0, pressure: 0)])
}

/// The click count of a press that comes `seconds` after a press and a release of a button (the
/// numbers are fractions of a power of two, so that the sums of the clock are exact).
func secondPress(
  after seconds: Double, at second: (Double, Double) = (300, 200),
  first firstButton: PointerButton = .left, then secondButton: PointerButton = .left
) -> Int? {
  let (translator, clock) = translatorWithAClock()
  _ = translator.events(for: pointer(.down, button: firstButton))
  _ = translator.events(for: pointer(.up, button: firstButton))
  clock.seconds += seconds
  let events = translator.events(for: pointer(.down, at: second, button: secondButton))
  if case .mouse(let event)? = events?.first { return event.clickCount }
  return nil
}

func testPressesCountAsClicksWhenTheyComeSoonAndCloseToEachOther() {
  check(secondPress(after: 0.25) == 2, "soon and close")
  check(secondPress(after: 0.25, at: (302, 203)) == 2)
  // The time and the place are measured from the press before, and their edges still count.
  check(secondPress(after: 0.5) == 2, "exactly the time of a double click")
  check(secondPress(after: 0.5625) == 1, "too late")
  check(secondPress(after: 0) == 2, "the same moment")
  check(secondPress(after: 0.25, at: (304, 204)) == 2, "exactly the slop, across and down")
  check(secondPress(after: 0.25, at: (296, 196)) == 2, "and up and to the left")
  check(secondPress(after: 0.25, at: (304.0625, 200)) == 1, "too far across")
  check(secondPress(after: 0.25, at: (300, 204.0625)) == 1, "too far down")
  check(secondPress(after: 0.25, at: (295.9375, 200)) == 1, "too far to the left")
  check(secondPress(after: 0.25, at: (300, 195.9375)) == 1, "too far up")
  check(secondPress(after: 0.25, then: .right) == 1, "another button has its own count")
  check(secondPress(after: -1) == 1, "a clock that went back")
}

func testClicksInARowGoOnCountingUntilOneComesTooLate() {
  let (translator, clock) = translatorWithAClock()
  func press(_ button: PointerButton = .left) -> Int? {
    let down = translator.events(for: pointer(.down, button: button))
    _ = translator.events(for: pointer(.up, button: button))
    if case .mouse(let event)? = down?.first { return event.clickCount }
    return nil
  }
  check(press() == 1)
  clock.seconds += 0.25
  check(press() == 2)
  clock.seconds += 0.25
  check(press() == 3)
  clock.seconds += 0.25
  check(press() == 4, "a fourth click goes on counting")
  clock.seconds += 0.75
  check(press() == 1, "too late")
  clock.seconds += 0.25
  check(press() == 2)
  clock.seconds += 0.25
  check(press(.right) == 1, "another button")
  clock.seconds += 0.25
  check(press(.left) == 1, "the press of the other button ended the row")

  // A press that never stops being repeated does not count past what a Mac holds.
  var last = 0
  for _ in 0..<1_100 {
    clock.seconds += 0.0078125
    last = max(last, press() ?? 0)
  }
  check(last == 1_000, "\(last)")
}

func testADragAndTheReleaseOfAClickCarryItsCount() {
  let (translator, clock) = translatorWithAClock()
  _ = translator.events(for: pointer(.down))
  _ = translator.events(for: pointer(.up))
  clock.seconds += 0.2
  _ = translator.events(for: pointer(.down))
  check(
    translator.events(for: pointer(.move, at: (310, 200)))
      == [mouse(.dragged, at: (310, 200), count: 2, pressure: 1)])
  check(
    translator.events(for: pointer(.up, at: (320, 200)))
      == [mouse(.up, at: (320, 200), count: 2, pressure: 0)])
  // A release of a button that was never pressed here is a single click.
  check(
    translator.events(for: pointer(.up, button: .right))
      == [mouse(.up, button: .right, count: 1, pressure: 0)])
}

func testAPointerAtAPlaceThatIsNotANumberMakesNoEvent() {
  let (translator, _) = translatorWithAClock()
  for action in [PointerAction.move, .down, .up, .scroll] {
    check(translator.events(for: pointer(action, at: (.nan, 1))) == nil, "\(action) x")
    check(translator.events(for: pointer(action, at: (1, .infinity))) == nil, "\(action) y")
  }
}

func scrolled(_ at: (Double, Double) = (300, 200), dx: Int32, dy: Int32, modifiers: InputModifiers = [])
  -> SyntheticEvent
{
  .scroll(
    SyntheticScroll(at: ScreenPoint(x: at.0, y: at.1), dx: dx, dy: dy, modifiers: modifiers))
}

func testAScrollMovesByWholePixelsAndKeepsWhatIsLeft() {
  let (translator, _) = translatorWithAClock()
  // Less than a pixel moves nothing, and is not lost. (The numbers are fractions of a power of
  // two, so that the sums are exact.)
  check(translator.events(for: pointer(.scroll, dx: 0.25, dy: 0.75)) == [])
  check(
    translator.events(for: pointer(.scroll, dx: 0.25, dy: 0.75, modifiers: [.option]))
      == [scrolled(dx: 0, dy: 1, modifiers: [.option])])
  check(translator.events(for: pointer(.scroll, dx: 0.5, dy: 0.25)) == [scrolled(dx: 1, dy: 0)])
  // The direction is the finger's: a positive dx moves the content to the right, and a positive dy
  // moves it down. What is left of a move in one direction is taken from the next one.
  check(
    translator.events(for: pointer(.scroll, at: (5, 6), dx: -2.5, dy: 7.5))
      == [scrolled((5, 6), dx: -2, dy: 8)])
  check(translator.events(for: pointer(.scroll, dx: -0.25, dy: 0)) == [])
  check(translator.events(for: pointer(.scroll, dx: -0.5, dy: 0)) == [scrolled(dx: -1, dy: 0)])
  check(translator.events(for: pointer(.scroll, dx: 0, dy: 0)) == [], "nothing moved")
}

func testAScrollIsBoundedAndMustBeANumber() {
  let (translator, _) = translatorWithAClock()
  check(
    translator.events(for: pointer(.scroll, dx: 50_000, dy: -50_000))
      == [scrolled(dx: 10_000, dy: -10_000)])
  // What was cut is not saved up for the next step.
  check(translator.events(for: pointer(.scroll, dx: 0.5, dy: 0.5)) == [])
  check(translator.events(for: pointer(.scroll, dx: .nan, dy: 0)) == nil)
  check(translator.events(for: pointer(.scroll, dx: 0, dy: .infinity)) == nil)
  check(InputTranslator.mostScroll == 10_000)
}

// MARK: - A window that is steered

func testAWindowIsSteeredByHandingEachEventToItsPoster() {
  let listing = FakeListing()
  listing.described = .window(listed(sharedWindow, process: sharedProcess))
  let activity = FakeActivity()
  let poster = FakePoster()
  let (translator, _) = translatorWithAClock()
  let control = WindowControl(
    reader: SurroundingsReader(
      window: sharedWindow, process: sharedProcess, listing: listing, activity: activity,
      layers: testLayers),
    translator: translator, poster: poster)

  check(control.surroundings()?.onScreen == true)

  check(control.post(key(0x04)))
  check(poster.posted == [syntheticKey(0x00)])

  poster.posted = []
  check(control.post(key(0, text: "é")), "text goes down and up")
  check(poster.posted == [syntheticKey(0, text: [233]), syntheticKey(0, down: false, text: [233])])

  poster.posted = []
  check(control.post(key(0xE1)), "a modifier key is handled by making nothing")
  check(poster.posted.isEmpty)

  check(!control.post(key(0x66)), "a key that has no code on a Mac is a failure")
  check(poster.posted.isEmpty)
}

func testAnEventThatCannotBeHandedOverIsAFailureAndTheRestOfThePostIsNotTried() {
  let listing = FakeListing()
  let poster = FakePoster()
  let (translator, _) = translatorWithAClock()
  let control = WindowControl(
    reader: SurroundingsReader(
      window: sharedWindow, process: sharedProcess, listing: listing, activity: FakeActivity(),
      layers: testLayers),
    translator: translator, poster: poster)
  poster.works = [false]
  check(!control.post(key(0, text: "x")))
  check(poster.posted.count == 1, "\(poster.posted.count)")
  poster.posted = []
  poster.works = [true, false]
  check(!control.post(key(0, text: "x")))
  check(poster.posted.count == 2)
}

func testADisplayIsSteeredByAskingTheDisplayAndHandingEachEventToItsPoster() {
  let displays = FakeDisplays()
  displays.places[sharedDisplay] = displayBounds
  let poster = FakePoster()
  let (translator, _) = translatorWithAClock()
  let control = WindowControl(
    reader: DisplayReader(display: sharedDisplay, listing: displays, activity: FakeActivity()),
    translator: translator, poster: poster)

  check(
    control.surroundings()
      == WindowSurroundings(
        locked: false, onScreen: true, keyboard: .shared, window: displayBounds, fronts: []))
  check(displays.asked == [sharedDisplay])

  check(control.post(pointer(.move, at: (700, 400))))
  check(control.post(key(0x04)))
  check(poster.posted.count == 2)
  check(poster.posted[1] == syntheticKey(0x00))
  check(!control.post(key(0x66)), "a key that has no code on a Mac is a failure")
  poster.works = [false]
  check(!control.post(key(0, text: "x")), "a poster that cannot is a failure")
}

// MARK: - The surroundings of a whole display

func readerOfASharedDisplay(
  _ setUp: (FakeDisplays, FakeActivity) -> Void = { _, _ in }
) -> (DisplayReader, FakeDisplays, FakeActivity) {
  let displays = FakeDisplays()
  displays.places[sharedDisplay] = displayBounds
  let activity = FakeActivity()
  setUp(displays, activity)
  return (
    DisplayReader(display: sharedDisplay, listing: displays, activity: activity), displays, activity
  )
}

func testTheSurroundingsOfADisplayAreItsPlaceWithTheKeyboardAndNothingInFront() {
  // The application that is in front is not the display's, and does not matter: everything on the
  // display is shared.
  let (reader, displays, activity) = readerOfASharedDisplay { _, activity in
    activity.frontmostProcess = 999
  }
  check(
    reader.read()
      == WindowSurroundings(
        locked: false, onScreen: true, keyboard: .shared, window: displayBounds, fronts: []))
  check(displays.asked == [sharedDisplay])
  check(activity.brought.isEmpty, "no application is brought to the front for a display")
}

func testADisplayLeftOfTheMainOneKeepsItsPlaceBelowZero() {
  let place = ScreenRect(x: -1920, y: -120, width: 1920, height: 1080)
  let (reader, _, _) = readerOfASharedDisplay { displays, _ in
    displays.places[sharedDisplay] = place
  }
  check(reader.read()?.window == place, "\(String(describing: reader.read()))")
}

func testALockedMacIsToldWithoutAskingForTheDisplay() {
  let (reader, displays, _) = readerOfASharedDisplay { _, activity in activity.locked = true }
  check(
    reader.read()
      == WindowSurroundings(
        locked: true, onScreen: false, keyboard: .unknown, window: .zero, fronts: []))
  check(displays.asked.isEmpty, "a locked Mac is not asked about its displays")
}

func testADisplayThatIsNotThereIsNotOnTheScreen() {
  let (reader, _, _) = readerOfASharedDisplay { displays, _ in displays.places = [:] }
  check(
    reader.read()
      == WindowSurroundings(
        locked: false, onScreen: false, keyboard: .unknown, window: .zero, fronts: []))
}

// MARK: - The two functions of the table

/// The window that the person picked, with the table in which it was registered.
func steerableWindow(_ window: FakeSharedWindow) -> (BatonSharerVTable, UnsafeMutableRawPointer?)? {
  guard let table = registeredSharing(FakeSharerPlatform()) else { return nil }
  return (table, chosen(window))
}

func testTheTableHasTheTwoFunctionsOfInput() {
  guard let table = registeredSharing(FakeSharerPlatform()) else { return check(false) }
  check(table.surroundings != nil && table.post != nil)
  check(table.size == UInt32(MemoryLayout<BatonSharerVTable>.size))
}

/// Asks for the surroundings the way the Rust side does: the struct has room for `cap` windows
/// and is filled with the careful answer, so that what the platform leaves alone shows.
func askForSurroundings(
  _ table: BatonSharerVTable, _ context: UnsafeMutableRawPointer?, cap: Int,
  withRoom: Bool = true, size: UInt32 = UInt32(MemoryLayout<BatonSurroundings>.size)
) -> (code: Int32, around: BatonSurroundings, fronts: [BatonFront]) {
  let room = UnsafeMutablePointer<BatonFront>.allocate(capacity: max(cap, 1))
  defer { room.deallocate() }
  room.initialize(
    repeating: BatonFront(
      rect: BatonRect(x: -1, y: -1, width: -1, height: -1), flags: 99, reserved: 0),
    count: max(cap, 1))
  var around = BatonSurroundings(
    size: size, locked: 1, on_screen: 0, keyboard: UInt32(BATON_KEYBOARD_UNKNOWN),
    window: BatonRect(x: -1, y: -1, width: -1, height: -1), fronts: withRoom ? room : nil,
    front_cap: UInt32(cap), front_count: 0)
  let code = table.surroundings?(context, &around) ?? -99
  let written = (0..<min(Int(around.front_count), max(cap, 1))).map { room[$0] }
  return (code, around, written)
}

func testTheSurroundingsOfAWindowAreWrittenWhereTheRustSideLeftRoomForThem() {
  let window = FakeSharedWindow()
  guard let (table, context) = steerableWindow(window) else { return check(false) }
  window.around = WindowSurroundings(
    locked: false, onScreen: true, keyboard: .shared,
    window: ScreenRect(x: 100, y: 200, width: 640, height: 480),
    fronts: [
      WindowFront(rect: ScreenRect(x: 1, y: 2, width: 3, height: 4), sameApplication: true, menu: false),
      WindowFront(rect: ScreenRect(x: 5, y: 6, width: 7, height: 8), sameApplication: false, menu: true),
      WindowFront(rect: ScreenRect(x: 9, y: 10, width: 11, height: 12), sameApplication: true, menu: true),
      WindowFront(rect: ScreenRect(x: 13, y: 14, width: 15, height: 16), sameApplication: false, menu: false),
    ])
  let asked = askForSurroundings(table, context, cap: 6)
  check(asked.code == 0, "code \(asked.code)")
  check(asked.around.locked == 0 && asked.around.on_screen == 1)
  check(asked.around.keyboard == BATON_KEYBOARD_SHARED)
  check(
    asked.around.window.x == 100 && asked.around.window.y == 200
      && asked.around.window.width == 640 && asked.around.window.height == 480)
  check(asked.around.front_count == 4 && asked.around.front_cap == 6)
  check(asked.fronts.count == 4)
  let flags = asked.fronts.map { $0.flags }
  check(
    flags == [
      BATON_FRONT_SAME_APPLICATION, BATON_FRONT_MENU,
      BATON_FRONT_SAME_APPLICATION | BATON_FRONT_MENU, 0,
    ], "\(flags)")
  check(
    asked.fronts.map { [$0.rect.x, $0.rect.y, $0.rect.width, $0.rect.height] } == [
      [1, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12], [13, 14, 15, 16],
    ])
  check(asked.fronts.allSatisfy { $0.reserved == 0 })

  // The other answers about the keyboard, and a window that is locked and not seen.
  for (focus, number) in [
    (KeyboardFocus.elsewhere, BATON_KEYBOARD_ELSEWHERE), (.shared, BATON_KEYBOARD_SHARED),
    (.unknown, BATON_KEYBOARD_UNKNOWN),
  ] {
    window.around = WindowSurroundings(
      locked: true, onScreen: false, keyboard: focus, window: .zero, fronts: [])
    let again = askForSurroundings(table, context, cap: 0, withRoom: false)
    check(again.code == 0 && again.around.keyboard == number, "\(focus)")
    check(again.around.locked == 1 && again.around.on_screen == 0 && again.around.front_count == 0)
  }
  table.release?(context)
}

func testSurroundingsThatCannotBeSaidOrDoNotFitAreAFailureAndLeaveTheStructAlone() {
  let window = FakeSharedWindow()
  guard let (table, context) = steerableWindow(window) else { return check(false) }
  window.around = nil
  var asked = askForSurroundings(table, context, cap: 4)
  check(asked.code == BATON_E_PLATFORM, "code \(asked.code)")
  check(asked.around.locked == 1 && asked.around.on_screen == 0 && asked.around.front_count == 0)
  check(asked.around.keyboard == BATON_KEYBOARD_UNKNOWN)

  let front = WindowFront(rect: ScreenRect(x: 1, y: 2, width: 3, height: 4), sameApplication: false, menu: false)
  let around = { (count: Int) in
    WindowSurroundings(
      locked: false, onScreen: true, keyboard: .shared, window: sharedBounds,
      fronts: Array(repeating: front, count: count))
  }
  // More windows than there is room for.
  window.around = around(5)
  asked = askForSurroundings(table, context, cap: 4)
  check(asked.code == BATON_E_PLATFORM, "code \(asked.code)")
  check(asked.around.front_count == 0 && asked.around.on_screen == 0)
  check(asked.fronts.isEmpty)
  window.around = around(4)
  check(askForSurroundings(table, context, cap: 4).code == 0)
  // More than the interface allows, whatever room the Rust side offers.
  window.around = around(Int(BATON_FRONTS_MAX) + 1)
  check(askForSurroundings(table, context, cap: 1000).code == BATON_E_PLATFORM)
  window.around = around(Int(BATON_FRONTS_MAX))
  check(askForSurroundings(table, context, cap: 1000).code == 0)
  // Windows to write and nowhere to write them.
  window.around = around(1)
  asked = askForSurroundings(table, context, cap: 4, withRoom: false)
  check(asked.code == BATON_E_PLATFORM)
  window.around = around(0)
  check(askForSurroundings(table, context, cap: 0, withRoom: false).code == 0)
  table.release?(context)
}

func testSurroundingsAreRefusedForAStructThatIsTooShortOrAWindowThatIsNotThere() {
  let window = FakeSharedWindow()
  guard let (table, context) = steerableWindow(window) else { return check(false) }
  window.around = WindowSurroundings(
    locked: false, onScreen: true, keyboard: .shared, window: sharedBounds, fronts: [])
  let small = UInt32(MemoryLayout<BatonSurroundings>.size) - 1
  check(askForSurroundings(table, context, cap: 0, withRoom: false, size: small).code == BATON_E_ARGUMENT)
  let arena = Arena()
  let short = arena.shortStruct(BatonSurroundings.self, size: 16)
  check(table.surroundings?(context, short) == BATON_E_ARGUMENT)
  check(table.surroundings?(context, nil) == BATON_E_ARGUMENT)
  var around = BatonSurroundings(
    size: UInt32(MemoryLayout<BatonSurroundings>.size), locked: 1, on_screen: 0,
    keyboard: UInt32(BATON_KEYBOARD_UNKNOWN), window: BatonRect(x: 0, y: 0, width: 0, height: 0),
    fronts: nil, front_cap: 0, front_count: 0)
  check(table.surroundings?(nil, &around) == BATON_E_ARGUMENT)
  table.release?(context)
}

/// A post the way the Rust side builds it.
func batonPost(
  kind: UInt32, x: Double = 0, y: Double = 0, action: UInt32 = 0, button: UInt32 = 0,
  dx: Float = 0, dy: Float = 0, modifiers: UInt32 = 0, code: UInt32 = 0, down: UInt32 = 0,
  repeat again: UInt32 = 0, text: BatonBytes = BatonBytes(ptr: nil, len: 0)
) -> BatonPost {
  BatonPost(
    size: UInt32(MemoryLayout<BatonPost>.size), kind: kind, x: x, y: y, action: action,
    button: button, dx: dx, dy: dy, modifiers: modifiers, code: code, down: down, repeat: again,
    text: text)
}

func put(
  _ table: BatonSharerVTable, _ context: UnsafeMutableRawPointer?, _ post: BatonPost
) -> Int32 {
  var copy = post
  return table.post?(context, &copy) ?? -99
}

func testAPointerThatTheRustSidePutsToTheWindowIsReadAsItWasMeant() {
  let window = FakeSharedWindow()
  guard let (table, context) = steerableWindow(window) else { return check(false) }
  let actions: [(UInt32, PointerAction)] = [
    (BATON_POINTER_MOVE, .move), (BATON_POINTER_DOWN, .down), (BATON_POINTER_UP, .up),
    (BATON_POINTER_SCROLL, .scroll),
  ]
  let buttons: [(UInt32, PointerButton)] = [
    (BATON_BUTTON_LEFT, .left), (BATON_BUTTON_RIGHT, .right), (BATON_BUTTON_MIDDLE, .middle),
  ]
  for (number, action) in actions {
    for (code, button) in buttons {
      window.posts = []
      let post = batonPost(
        kind: BATON_POST_POINTER, x: 123.5, y: -4.25, action: number, button: code, dx: 1.5,
        dy: -2.5, modifiers: BATON_MOD_SHIFT | BATON_MOD_COMMAND)
      check(put(table, context, post) == 0)
      check(
        window.posts == [
          .pointer(
            PointerPost(
              at: ScreenPoint(x: 123.5, y: -4.25), action: action, button: button, dx: 1.5, dy: -2.5,
              modifiers: [.shift, .command]))
        ], "\(action) \(button): \(window.posts)")
    }
  }
  table.release?(context)
}

func testAKeyThatTheRustSidePutsToTheWindowIsReadAsItWasMeant() {
  let window = FakeSharedWindow()
  guard let (table, context) = steerableWindow(window) else { return check(false) }
  let arena = Arena()
  let text = Array("Käse".utf8)
  let post = batonPost(
    kind: BATON_POST_KEY, modifiers: BATON_MOD_CONTROL | BATON_MOD_OPTION | BATON_MOD_CAPS_LOCK,
    code: 0x04, down: 1, repeat: 1, text: arena.batonBytes(text))
  check(put(table, context, post) == 0)
  check(
    window.posts == [
      .key(KeyPost(code: 0x04, down: true, repeating: true, modifiers: [.control, .option, .capsLock], text: "Käse"))
    ], "\(window.posts)")

  // Text of one byte is text.
  window.posts = []
  check(
    put(table, context, batonPost(kind: BATON_POST_KEY, down: 1, text: arena.batonBytes([0x61]))) == 0)
  check(
    window.posts == [.key(KeyPost(code: 0, down: true, repeating: false, modifiers: [], text: "a"))],
    "\(window.posts)")

  window.posts = []
  check(put(table, context, batonPost(kind: BATON_POST_KEY, code: 0x05, down: 0, repeat: 0)) == 0)
  check(
    window.posts == [
      .key(KeyPost(code: 0x05, down: false, repeating: false, modifiers: [], text: nil))
    ], "\(window.posts)")

  // A number that is not 0 or 1 is a yes, and a bit that is no modifier is no modifier.
  window.posts = []
  check(
    put(table, context, batonPost(kind: BATON_POST_KEY, modifiers: 0xFFFF_FFE0, code: 0x06, down: 7, repeat: 2)) == 0)
  check(
    window.posts == [
      .key(KeyPost(code: 0x06, down: true, repeating: true, modifiers: [], text: nil))
    ], "\(window.posts)")
  table.release?(context)
}

func testAPostThatIsNotWellFormedIsRefusedAndNeverReachesTheWindow() {
  let window = FakeSharedWindow()
  guard let (table, context) = steerableWindow(window) else { return check(false) }
  let arena = Arena()
  let refused: [(String, BatonPost)] = [
    ("a kind of a newer version", batonPost(kind: 3)),
    ("no kind", batonPost(kind: 0)),
    ("an action of a newer version", batonPost(kind: BATON_POST_POINTER, action: 5)),
    ("no action", batonPost(kind: BATON_POST_POINTER, action: 0)),
    ("a button of a newer version", batonPost(kind: BATON_POST_POINTER, action: BATON_POINTER_DOWN, button: 3)),
    (
      "text that is not UTF-8",
      batonPost(kind: BATON_POST_KEY, text: arena.batonBytes([0x61, 0xC3, 0x28]))
    ),
  ]
  for (what, post) in refused {
    check(put(table, context, post) == BATON_E_ARGUMENT, what)
  }
  let short = arena.shortStruct(BatonPost.self, size: 60)
  check(table.post?(context, short) == BATON_E_ARGUMENT, "a short struct")
  // A struct that says it is short is refused, though it is whole and holds a post that is fine,
  // and one that says it is longer than this version knows is read as far as this version knows.
  var fine = batonPost(kind: BATON_POST_KEY, code: 4, down: 1)
  let whole = UInt32(MemoryLayout<BatonPost>.size)
  for size in [0, 4, whole - 1] {
    fine.size = size
    check(put(table, context, fine) == BATON_E_ARGUMENT, "a struct of \(size) bytes")
  }
  check(window.posts.isEmpty, "\(window.posts)")
  fine.size = whole
  check(put(table, context, fine) == 0, "a struct of exactly its size")
  fine.size = whole + 8
  check(put(table, context, fine) == 0, "a struct of a newer version")
  check(window.posts.count == 2, "\(window.posts)")
  window.posts = []
  check(table.post?(context, nil) == BATON_E_ARGUMENT)
  check(table.post?(nil, nil) == BATON_E_ARGUMENT)
  fine = batonPost(kind: BATON_POST_KEY, code: 4, down: 1)
  check(table.post?(nil, &fine) == BATON_E_ARGUMENT, "no window")
  check(window.posts.isEmpty, "\(window.posts)")

  // A window that could not put it to the Mac is a failure of that event.
  window.postWorks = false
  check(put(table, context, batonPost(kind: BATON_POST_KEY, code: 4, down: 1)) == BATON_E_PLATFORM)
  check(window.posts.count == 1)
  table.release?(context)
}

func testAWindowThatCannotBeSteeredSaysNothingAndPutsNothing() {
  // The defaults of the protocol are what a window of a view-only version has.
  final class ViewOnly: SharedWindow {
    func prepare(takes: ViewerTakes) throws -> PictureFormat { PictureFormat(width: 2, height: 2, fps: 1) }
    func start(link: UInt64) throws {}
    func ended() {}
    func released() {}
  }
  check(ViewOnly().surroundings() == nil)
  check(!ViewOnly().post(.key(KeyPost(code: 4, down: true, repeating: false, modifiers: [], text: nil))))
}

// MARK: - What the library is told

func testAPictureCarriesTheGeometryOfItsWindowToTheLibrary() {
  withSession { session in
    let geometry = PictureGeometry(
      windowWidth: 800.5, windowHeight: 600.25, pictureWidth: 1600, pictureHeight: 1200,
      content: ScreenRect(x: 10, y: 20, width: 1500, height: 1100))
    check(
      session.send(
        EncodedPicture(
          sample: Data([0, 0, 0, 1, 0x41]), lengthSize: 4, sets: [], captureTime: 9,
          geometry: geometry)) == .sent)
    check(
      session.send(
        EncodedPicture(sample: Data([0, 0, 0, 1, 0x41]), lengthSize: 4, sets: [], captureTime: 10))
        == .sent)
    stubLock.lock()
    let calls = sourceCalls
    stubLock.unlock()
    guard calls.count == 2, case .sent(_, let first) = calls[0], case .sent(_, let second) = calls[1]
    else { return check(false, "\(calls)") }
    check(
      first.geometry
        == SentGeometry(
          windowWidth: 800.5, windowHeight: 600.25, pictureWidth: 1600, pictureHeight: 1200,
          content: [10, 20, 1500, 1100]), "\(String(describing: first.geometry))")
    check(second.geometry == nil, "a picture with no geometry has all zero")
  }
}

func testAChangeAPauseAndAResumeReachTheLibraryAboutTheLink() {
  withSession { session in
    session.changed(1_234)
    session.pause(.locked)
    session.pause(.hidden)
    session.resume()
    stubLock.lock()
    let calls = sourceCalls
    stubLock.unlock()
    check(
      calls == [
        .changed(55, 1_234), .pause(55, BATON_PAUSE_LOCKED), .pause(55, BATON_PAUSE_HIDDEN),
        .resume(55),
      ], "\(calls)")
  }
}

// MARK: - The pump

func geometryOfAnImage(_ number: Int) -> PictureGeometry {
  PictureGeometry(
    windowWidth: Double(100 + number), windowHeight: 50, pictureWidth: 200, pictureHeight: 100,
    content: ScreenRect(x: 0, y: 0, width: 200, height: 100))
}

func testEveryImageTheCaptureHandsOverIsToldAsAChangeBeforeAPictureIsAskedFor() {
  let session = FakeSession()
  session.begins = [.key, .notNow, .delta]
  let pump = FramePump(session: session, encoder: FakeEncoder())
  for (number, time) in [(1, UInt64(1_000)), (2, 2_500), (3, 4_000)] {
    session.micros = time
    pump.captured(Img(number))
  }
  check(session.changes == [1_000, 2_500, 4_000], "even the image that was not made: \(session.changes)")
  check(
    session.calls == [
      "clock", "changed", "begin", "send", "clock", "changed", "begin", "clock", "changed",
      "begin", "send",
    ], "\(session.calls)")
}

func testNoChangeIsToldOnceTheSessionIsOver() {
  let session = FakeSession()
  session.alive = false
  let pump = FramePump(session: session, encoder: FakeEncoder())
  pump.captured(Img(1))
  check(session.changes.isEmpty && pump.over, "\(session.changes)")

  let ended = FakeSession()
  ended.begins = [.over]
  let second = FramePump(session: ended, encoder: FakeEncoder())
  second.captured(Img(1))
  check(ended.changes == [1_000] && second.over, "the change that came before the end")
  second.captured(Img(2))
  check(ended.changes == [1_000], "none after it")
}

func testAPictureHasTheGeometryOfTheImageItWasMadeFrom() {
  let session = FakeSession()
  session.begins = [.key, .delta, .delta, .key, .key]
  let encoder = FakeEncoder()
  let pump = FramePump(session: session, encoder: encoder)
  pump.captured(Img(1), geometry: geometryOfAnImage(1))
  pump.captured(Img(2), geometry: geometryOfAnImage(2))
  pump.captured(Img(3))
  check(
    session.sent.map { $0.geometry } == [geometryOfAnImage(1), geometryOfAnImage(2), nil],
    "\(session.sent.map { $0.geometry })")

  // The same image made into a picture again has the geometry it had, and an image without one
  // does not borrow the geometry of the one before.
  pump.captured(Img(4), geometry: geometryOfAnImage(4))
  session.pending = true
  pump.tick()
  check(
    session.sent.suffix(2).map { $0.geometry } == [geometryOfAnImage(4), geometryOfAnImage(4)],
    "\(session.sent.map { $0.geometry })")
}

func testAPictureThatTheEncoderMadeKeepsItsOwnFields() {
  let session = FakeSession()
  session.begins = [.key]
  let pump = FramePump(session: session, encoder: FakeEncoder())
  session.micros = 5_000
  pump.captured(Img(9), geometry: geometryOfAnImage(9))
  guard let picture = session.sent.first else { return check(false) }
  check(picture.captureTime == 5_000 && picture.lengthSize == 4 && picture.sets.count == 2)
  check(picture.sample == Data([9, 1]), "\(picture.sample)")
}

// MARK: - The consent to steer a window

func testTheConsentIsToldToTheAppWithTheWordsItReads() {
  // The screens of the app read these words (apps/baton/lib/src/control_access.dart).
  check(ControlAccess.allowed.rawValue == "allowed")
  check(ControlAccess.needed.rawValue == "needed")
  check(ControlAccess.unavailable.rawValue == "unavailable")
  check(ControlAccess(rawValue: "needed") == .needed)
  check(ControlAccess(rawValue: "") == nil)
}

// MARK: - Places on the screen

func testARectangleCanHoldSomethingOnlyWithAFiniteSizeAboveZero() {
  func usable(_ x: Double, _ y: Double, _ width: Double, _ height: Double) -> Bool {
    ScreenRect(x: x, y: y, width: width, height: height).isUsable
  }
  check(usable(10, 20, 30, 40))
  check(usable(-10, -20, 30, 40), "a window can lie left of and above the main display")
  check(usable(0, 0, 0.5, 0.5), "a size below a point is a size")
  check(!usable(0, 0, 0, 40), "no width")
  check(!usable(0, 0, 30, 0), "no height")
  check(!usable(0, 0, -30, 40), "a width below zero")
  check(!usable(0, 0, 30, -40), "a height below zero")
  for bad in [Double.nan, .infinity, -.infinity] {
    check(!usable(bad, 0, 30, 40), "x is \(bad)")
    check(!usable(0, bad, 30, 40), "y is \(bad)")
    check(!usable(0, 0, bad, 40), "width is \(bad)")
    check(!usable(0, 0, 30, bad), "height is \(bad)")
  }
}

func testARectangleEndsWhereItsSizeTakesIt() {
  let rect = ScreenRect(x: 10, y: 20, width: 30, height: 40)
  check(rect.right == 40 && rect.bottom == 60, "\(rect.right) \(rect.bottom)")
  let before = ScreenRect(x: -50, y: -60, width: 20, height: 10)
  check(before.right == -30 && before.bottom == -50, "\(before.right) \(before.bottom)")
}

func testTwoRectanglesOverlapOnlyWhenTheyShareSomeArea() {
  let base = ScreenRect(x: 100, y: 100, width: 100, height: 100)
  /// Whether they overlap, which must not depend on which one is asked.
  func overlap(_ x: Double, _ y: Double, _ width: Double = 100, _ height: Double = 100) -> Bool? {
    let other = ScreenRect(x: x, y: y, width: width, height: height)
    let one = base.overlaps(other)
    return one == other.overlaps(base) ? one : nil
  }
  check(overlap(150, 150) == true, "across a corner")
  check(overlap(100, 100) == true, "the same place")
  check(overlap(120, 120, 10, 10) == true, "inside")
  check(overlap(50, 120, 300, 10) == true, "across, a strip that goes through")
  check(overlap(120, 50, 10, 300) == true, "down, a strip that goes through")
  check(overlap(199, 150) == true, "a point of the right edge")
  check(overlap(1, 150) == true, "a point of the left edge")
  check(overlap(150, 199) == true, "a point of the bottom edge")
  check(overlap(150, 1) == true, "a point of the top edge")
  check(overlap(200, 150) == false, "touching on the right")
  check(overlap(0, 150) == false, "touching on the left")
  check(overlap(150, 200) == false, "touching below")
  check(overlap(150, 0) == false, "touching above")
  check(overlap(250, 150) == false, "apart on the right")
  check(overlap(150, 250) == false, "apart below")
  check(overlap(-50, 150) == false, "apart on the left")
  check(overlap(150, -50) == false, "apart above")
  check(overlap(250, 250) == false, "apart on both")
  check(overlap(150, 250, 10, 10) == false, "in the same columns and not in the same rows")
  check(overlap(250, 150, 10, 10) == false, "in the same rows and not in the same columns")
}
