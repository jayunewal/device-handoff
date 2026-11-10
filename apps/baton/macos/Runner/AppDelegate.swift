import Cocoa
import FlutterMacOS

@main
class AppDelegate: FlutterAppDelegate {
  /// The item that Baton keeps in the menu bar for as long as it runs. The app holds it, because
  /// the menu bar drops an item that nothing refers to.
  private var menuBarItem: NSStatusItem?

  /// The claim that Baton makes on the system for as long as it runs, so that the system does not
  /// slow it down (see `holdOffAppNap`).
  private var running: NSObjectProtocol?

  /// Whether the window is on the screen but cannot be seen, because a login item opened Baton
  /// and nobody asked for the window (see `startHidden`).
  private var windowWaitsToHide = false

  /// Baton keeps running when its window is closed. Other devices reach this Mac through it, and a
  /// Baton that quit with its window could be reached only while somebody sat at the Mac. The
  /// window hides instead (`MainFlutterWindow.performClose`), and the Dock, the item in the menu
  /// bar and a second start of the app bring it back. Baton ends only where the person quits it.
  override func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    return false
  }

  override func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
    return true
  }

  override func applicationDidFinishLaunching(_ notification: Notification) {
    super.applicationDidFinishLaunching(notification)
    holdOffAppNap()
    addMenuBarItem()
    if wasOpenedAtSignIn() {
      startHidden()
    }
  }

  /// The Dock icon of a Baton that has no window on the screen, and a second start of the app
  /// from Finder, ask for the window. A window that waits to hide counts as one on the screen to
  /// the system, so it is shown here as well.
  override func applicationShouldHandleReopen(
    _ sender: NSApplication, hasVisibleWindows visible: Bool
  ) -> Bool {
    if !visible || windowWaitsToHide {
      showBaton()
    }
    return true
  }

  /// Brings the window to the front, wherever it is: hidden, minimized, or waiting to hide.
  func showBaton() {
    guard let window = mainFlutterWindow else { return }
    windowWaitsToHide = false
    window.alphaValue = 1
    window.ignoresMouseEvents = false
    if window.isMiniaturized {
      window.deminiaturize(nil)
    }
    window.makeKeyAndOrderFront(nil)
    if #available(macOS 14.0, *) {
      NSApp.activate()
    } else {
      NSApp.activate(ignoringOtherApps: true)
    }
  }

  /// The app says that its first screen is up (lib/src/always_on.dart). A window that waited to
  /// hide goes now.
  func interfaceIsUp() {
    guard windowWaitsToHide, let window = mainFlutterWindow else { return }
    windowWaitsToHide = false
    window.orderOut(nil)
    window.alphaValue = 1
    window.ignoresMouseEvents = false
  }

  /// A Baton that a login item opened shows no window: the person has not asked for one, and
  /// Baton is there for the devices that reach it. But Flutter builds no screen while no window
  /// is on the screen, so a window that hid at once would leave the app on its first, empty
  /// screen, with nothing listening for what other devices send. The window therefore stays on
  /// the screen, with no color and out of the way of the pointer, until the app says that it is
  /// up (`interfaceIsUp`). A start that goes wrong never says so, and the window comes after a
  /// minute, so that the person sees what went wrong.
  private func startHidden() {
    guard let window = mainFlutterWindow else { return }
    windowWaitsToHide = true
    window.alphaValue = 0
    window.ignoresMouseEvents = true
    DispatchQueue.main.asyncAfter(deadline: .now() + 60) { [weak self] in
      if self?.windowWaitsToHide == true {
        self?.showBaton()
      }
    }
  }

  /// Whether a login item opened this run of the app. The system says so in the event that opens
  /// the app, and in no other way.
  private func wasOpenedAtSignIn() -> Bool {
    guard let event = NSAppleEventManager.shared().currentAppleEvent else { return false }
    let opensApp = UInt32(event.eventID) == UInt32(kAEOpenApplication)
    let launchedBy = event.paramDescriptor(forKeyword: AEKeyword(keyAEPropData))?.enumCodeValue
    return opensApp && launchedBy == OSType(keyAELaunchedAsLogInItem)
  }

  /// The system slows down an app that has no window on the screen and that the person is not
  /// using ("App Nap"): its timers run late and its network work waits. Baton is then slow to
  /// answer the devices that talk to it. This asks the system not to, and leaves it free to put
  /// the Mac to sleep when the person is away.
  private func holdOffAppNap() {
    running = ProcessInfo.processInfo.beginActivity(
      options: .userInitiatedAllowingIdleSystemSleep,
      reason: "Baton answers the devices that are paired with this Mac")
  }

  /// The item in the menu bar, which stays when the window is gone: it opens the window again and
  /// quits Baton.
  private func addMenuBarItem() {
    let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    if let button = item.button {
      // A symbol that an older system does not have is skipped, and the name stands in for it.
      let picture =
        NSImage(systemSymbolName: "laptopcomputer.and.iphone", accessibilityDescription: "Baton")
        ?? NSImage(systemSymbolName: "arrow.left.arrow.right", accessibilityDescription: "Baton")
      if let picture = picture {
        button.image = picture
      } else {
        button.title = "Baton"
      }
      button.toolTip = "Baton"
    }
    let menu = NSMenu()
    let open = menu.addItem(
      withTitle: "Open Baton", action: #selector(openBaton(_:)), keyEquivalent: "")
    open.target = self
    menu.addItem(NSMenuItem.separator())
    let quit = menu.addItem(
      withTitle: "Quit Baton", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "")
    quit.target = NSApp
    item.menu = menu
    menuBarItem = item
  }

  @objc private func openBaton(_ sender: Any?) {
    showBaton()
  }
}
