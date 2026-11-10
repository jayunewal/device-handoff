import AVFoundation
import Cocoa
import CoreServices
import FlutterMacOS
import ServiceManagement
import UserNotifications

class MainFlutterWindow: NSWindow {
  override func awakeFromNib() {
    keepBatonFolderOutOfBackups()

    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

    RegisterGeneratedPlugins(registry: flutterViewController)
    answerWhereReceivedFilesGo(messenger: flutterViewController.engine.binaryMessenger)
    showReceivedFilesInFinder(messenger: flutterViewController.engine.binaryMessenger)
    answerAboutNotifications(messenger: flutterViewController.engine.binaryMessenger)
    answerAboutPermissions(messenger: flutterViewController.engine.binaryMessenger)
    answerAboutStayingOn(messenger: flutterViewController.engine.binaryMessenger)
    answerAboutContinuing(messenger: flutterViewController.engine.binaryMessenger)

    super.awakeFromNib()
  }

  /// The close button and Command-W (the Close item of the Window menu) hide the window and leave
  /// Baton running, so that the devices that reach this Mac still can (AppDelegate.swift says
  /// why). A window in full screen leaves it first, and hides once it has left: hiding one there
  /// would leave its space empty, and the person who closed it asked for it to go, not to be
  /// asked again.
  override func performClose(_ sender: Any?) {
    if styleMask.contains(.fullScreen) {
      hideWhenOutOfFullScreen()
      toggleFullScreen(sender)
      return
    }
    orderOut(sender)
  }

  /// What waits for the window to leave full screen, to hide it then.
  private var fullScreenExit: NSObjectProtocol?

  private func hideWhenOutOfFullScreen() {
    guard fullScreenExit == nil else { return }
    fullScreenExit = NotificationCenter.default.addObserver(
      forName: NSWindow.didExitFullScreenNotification, object: self, queue: .main
    ) { [weak self] _ in
      guard let self = self else { return }
      if let waiting = self.fullScreenExit {
        NotificationCenter.default.removeObserver(waiting)
        self.fullScreenExit = nil
      }
      self.orderOut(nil)
    }
  }
}

/// Takes the person to the files that other devices sent: Finder opens with them selected, in the
/// folder that holds them. Finder shows them and does not open them. The names of the channel and
/// of the question must be the ones in lib/src/file_shower.dart.
private func showReceivedFilesInFinder(messenger: FlutterBinaryMessenger) {
  let channel = FlutterMethodChannel(
    name: "io.github.jayunewal.baton/files", binaryMessenger: messenger)
  channel.setMethodCallHandler { call, result in
    switch call.method {
    case "show":
      guard let paths = call.arguments as? [String] else {
        result(FlutterError(code: "bad_paths", message: "The paths were not text.", details: nil))
        return
      }
      // The person may have moved or deleted them since, and Finder would open the folder that
      // held them with nothing selected, so the app is told instead.
      let there = paths.filter { FileManager.default.fileExists(atPath: $0) }
        .map { URL(fileURLWithPath: $0) }
      if there.isEmpty {
        result(false)
        return
      }
      NSWorkspace.shared.activateFileViewerSelecting(there)
      result(true)
    default:
      result(FlutterMethodNotImplemented)
    }
  }
}

/// Shows a notification when a paired device unlocked this Mac, or tried to, and answers what the
/// system says about showing them. The notification names the device and says nothing of the
/// password. The names of the channel and of the questions, and the words of the answers, must be
/// the ones in lib/src/notice_access.dart.
private func answerAboutNotifications(messenger: FlutterBinaryMessenger) {
  let center = UNUserNotificationCenter.current()
  center.delegate = noticeDelegate
  let channel = FlutterMethodChannel(
    name: "io.github.jayunewal.baton/notices", binaryMessenger: messenger)
  channel.setMethodCallHandler { call, result in
    switch call.method {
    case "state":
      center.getNotificationSettings { settings in
        let answer = noticeAccess(settings.authorizationStatus)
        DispatchQueue.main.async { result(answer) }
      }
    case "request":
      // The system asks the person once, and says the same thing when asked again.
      center.requestAuthorization(options: [.alert]) { _, error in
        if let error = error {
          NSLog("Baton could not ask to show notifications: \(error)")
        }
        center.getNotificationSettings { settings in
          let answer = noticeAccess(settings.authorizationStatus)
          DispatchQueue.main.async { result(answer) }
        }
      }
    case "post":
      guard let arguments = call.arguments as? [String: String],
        let title = arguments["title"], let body = arguments["body"]
      else {
        result(
          FlutterError(
            code: "bad_notice", message: "The notice was not a title and a body.", details: nil))
        return
      }
      center.getNotificationSettings { settings in
        // The system takes a notification from an app that was told no, and shows nothing, so the
        // app is told that it was not shown.
        guard noticeAccess(settings.authorizationStatus) == "allowed" else {
          DispatchQueue.main.async { result(false) }
          return
        }
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        let request = UNNotificationRequest(
          identifier: UUID().uuidString, content: content, trigger: nil)
        center.add(request) { error in
          if let error = error {
            NSLog("Baton could not show a notification: \(error)")
          }
          DispatchQueue.main.async { result(error == nil) }
        }
      }
    default:
      result(FlutterMethodNotImplemented)
    }
  }
}

/// What the system says about showing notifications, in the words of lib/src/notice_access.dart.
private func noticeAccess(_ status: UNAuthorizationStatus) -> String {
  switch status {
  case .authorized, .provisional:
    return "allowed"
  case .notDetermined:
    return "needed"
  case .denied:
    return "denied"
  default:
    // A state that this version of the system adds, or one that only the iPhone has.
    return "unavailable"
  }
}

/// Lets a notification show while Baton is the app in front. The system shows none then unless the
/// app says to, and the person who comes back to the Mac and opens Baton is the one who most needs
/// to see that it was unlocked.
private final class NoticeDelegate: NSObject, UNUserNotificationCenterDelegate {
  func userNotificationCenter(
    _ center: UNUserNotificationCenter, willPresent notification: UNNotification,
    withCompletionHandler completionHandler:
      @escaping (UNNotificationPresentationOptions) -> Void
  ) {
    completionHandler([.banner, .list])
  }

  /// A click on a notification brings the window forward: what the notification tells of, the
  /// inbox or a question that waits for a yes, is there.
  func userNotificationCenter(
    _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
    withCompletionHandler completionHandler: @escaping () -> Void
  ) {
    DispatchQueue.main.async {
      if response.actionIdentifier == UNNotificationDefaultActionIdentifier {
        (NSApp.delegate as? AppDelegate)?.showBaton()
      }
      completionHandler()
    }
  }
}

/// The center keeps its delegate weakly, so this holds it for as long as the app runs.
private let noticeDelegate = NoticeDelegate()

/// Tells the app where the files that other devices send are put: the Downloads folder of the
/// person. The names of the channel and of the question must be the ones in
/// lib/src/support_folder.dart.
private func answerWhereReceivedFilesGo(messenger: FlutterBinaryMessenger) {
  let channel = FlutterMethodChannel(
    name: "io.github.jayunewal.baton/folder", binaryMessenger: messenger)
  channel.setMethodCallHandler { call, result in
    switch call.method {
    case "receiveFolder":
      do {
        let folder = try downloadsFolder().path
        result(folder)
      } catch {
        result(FlutterError(code: "no_folder", message: "\(error)", details: nil))
      }
    default:
      result(FlutterMethodNotImplemented)
    }
  }
}

/// The Downloads folder that the person looks in. The app is sandboxed, so the home folder it is
/// given is a folder of its own inside its container, and a Downloads folder made there is not
/// one the person would find. The entitlement com.apple.security.files.downloads.read-write lets
/// it use the real one, whose place is found from the user database. If that cannot be written
/// to, for want of the entitlement, the folder the system gives the app is used instead, so that
/// files still arrive.
private func downloadsFolder() throws -> URL {
  let files = FileManager.default
  if let entry = getpwuid(getuid()), let home = entry.pointee.pw_dir {
    let real = URL(fileURLWithPath: String(cString: home), isDirectory: true)
      .appendingPathComponent("Downloads", isDirectory: true)
    if canWrite(in: real) {
      return real
    }
  }
  return try files.url(
    for: .downloadsDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
}

/// Whether a file can be made in `folder`, which has to be there already.
private func canWrite(in folder: URL) -> Bool {
  var isFolder: ObjCBool = false
  guard FileManager.default.fileExists(atPath: folder.path, isDirectory: &isFolder),
    isFolder.boolValue
  else {
    return false
  }
  let probe = folder.appendingPathComponent(".baton-probe-\(UUID().uuidString)")
  guard FileManager.default.createFile(atPath: probe.path, contents: Data()) else {
    return false
  }
  try? FileManager.default.removeItem(at: probe)
  return true
}

/// Where Baton keeps its files, inside Library/Application Support. It holds the key that makes
/// this device what it is. The key must not leave the device, and a backup restored onto another
/// one would carry it there, so the folder is left out of backups. The name must be the one in
/// lib/src/rust_backend.dart.
private let batonFolderName = "baton-app"

private func keepBatonFolderOutOfBackups() {
  do {
    let support = try FileManager.default.url(
      for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
    var folder = support.appendingPathComponent(batonFolderName, isDirectory: true)
    // Private to this user, as Baton makes it itself. An existing folder is left as it is.
    try FileManager.default.createDirectory(
      at: folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
    var values = URLResourceValues()
    values.isExcludedFromBackup = true
    try folder.setResourceValues(values)
  } catch {
    NSLog("Baton could not keep its folder out of backups: \(error)")
  }
}

/// Says where the permissions that Baton keeps on this channel stand (Accessibility, Input
/// Monitoring, Screen Recording and the camera), asks for them, and opens the pane of System
/// Settings where a permission is turned on. The names of the channel, of the questions and of the permissions, and
/// the words of the answers, must be the ones in lib/src/permissions.dart.
private func answerAboutPermissions(messenger: FlutterBinaryMessenger) {
  let channel = FlutterMethodChannel(
    name: "io.github.jayunewal.baton/permissions", binaryMessenger: messenger)
  channel.setMethodCallHandler { call, result in
    let permission = call.arguments as? String ?? ""
    switch call.method {
    case "state":
      result(permissionState(permission))
    case "request":
      requestPermission(permission) { result(permissionState(permission)) }
    case "openSettings":
      guard let address = settingsAddress(permission) else {
        result(false)
        return
      }
      result(openSystemSettings(address))
    default:
      result(FlutterMethodNotImplemented)
    }
  }
}

/// Where a permission stands, in the words of lib/src/permissions.dart, or nothing for one that
/// this Mac cannot have, such as the camera of a Mac with none.
private func permissionState(_ permission: String) -> String? {
  switch permission {
  case "accessibility":
    // The same consent that lets the app hand input to other apps and hold the Mac's own.
    return CGPreflightPostEventAccess() ? "allowed" : "denied"
  case "inputMonitoring":
    return CGPreflightListenEventAccess() ? "allowed" : "denied"
  case "screenRecording":
    // Asked here, and not of the plugin that shares the screen, so that the answer does not
    // depend on whether the plugin is there.
    return CGPreflightScreenCaptureAccess() ? "allowed" : "denied"
  case "camera":
    guard AVCaptureDevice.default(for: .video) != nil else { return nil }
    switch AVCaptureDevice.authorizationStatus(for: .video) {
    case .authorized:
      return "allowed"
    case .notDetermined:
      return "needed"
    case .denied, .restricted:
      return "denied"
    @unknown default:
      return nil
    }
  default:
    return nil
  }
}

/// Asks the system for a permission. It asks the person once and says no without asking after
/// that, so the pane of System Settings where it is turned on is opened when the answer is no.
private func requestPermission(_ permission: String, then done: @escaping () -> Void) {
  switch permission {
  case "accessibility", "inputMonitoring", "screenRecording":
    // Not known to wait for the person, but nothing promises it, and the main thread is not held.
    DispatchQueue.global(qos: .userInitiated).async {
      let allowed =
        switch permission {
        case "accessibility": CGRequestPostEventAccess()
        case "inputMonitoring": CGRequestListenEventAccess()
        default: CGRequestScreenCaptureAccess()
        }
      DispatchQueue.main.async {
        if !allowed, let address = settingsAddress(permission) {
          _ = openSystemSettings(address)
        }
        done()
      }
    }
  case "camera":
    AVCaptureDevice.requestAccess(for: .video) { _ in DispatchQueue.main.async { done() } }
  default:
    done()
  }
}

/// Opens a pane of System Settings, and nothing else: an address that is not one of System
/// Settings is refused. This is the only place the Mac app asks the system to open anything; the
/// files that other devices send are only ever shown in Finder.
private func openSystemSettings(_ address: String) -> Bool {
  guard let url = URL(string: address), url.scheme == "x-apple.systempreferences" else {
    return false
  }
  return NSWorkspace.shared.open(url)
}

/// The address of the pane of System Settings where a permission is turned on.
private func settingsAddress(_ pane: String) -> String? {
  let privacy = "x-apple.systempreferences:com.apple.preference.security?"
  switch pane {
  case "accessibility":
    return privacy + "Privacy_Accessibility"
  case "inputMonitoring":
    return privacy + "Privacy_ListenEvent"
  case "screenRecording", "systemAudio":
    // One pane holds both: the sound has a list of its own at the foot of it.
    return privacy + "Privacy_ScreenCapture"
  case "camera":
    return privacy + "Privacy_Camera"
  case "localNetwork":
    return privacy + "Privacy_LocalNetwork"
  case "notifications":
    let id = Bundle.main.bundleIdentifier ?? ""
    return "x-apple.systempreferences:com.apple.Notifications-Settings.extension?id=\(id)"
  default:
    return nil
  }
}

/// Answers what the part of the app that keeps Baton running asks (AppDelegate.swift is the rest
/// of it): whether Baton opens when the person signs in to this Mac, turning that on or off, the
/// pane of System Settings where the person allows it, that the first screen of the app is up, and
/// whether the person can see the window. The names of the channel and of the questions, and the
/// words of the answers, must be the ones in lib/src/always_on.dart.
private func answerAboutStayingOn(messenger: FlutterBinaryMessenger) {
  let channel = FlutterMethodChannel(
    name: "io.github.jayunewal.baton/always_on", binaryMessenger: messenger)
  channel.setMethodCallHandler { call, result in
    switch call.method {
    case "ready":
      (NSApp.delegate as? AppDelegate)?.interfaceIsUp()
      result(nil)
    case "looking":
      result(personCanSeeBaton())
    case "signIn":
      result(signInState())
    case "setSignIn":
      guard let on = call.arguments as? Bool else {
        result(
          FlutterError(
            code: "bad_choice", message: "The choice was not a yes or a no.", details: nil))
        return
      }
      do {
        result(try setOpensAtSignIn(on))
      } catch {
        // What the system says, for the person to read: it is the system that knows why.
        result(
          FlutterError(code: "sign_in_failed", message: error.localizedDescription, details: nil))
      }
    case "openSignInSettings":
      result(openSignInSettings())
    default:
      result(FlutterMethodNotImplemented)
    }
  }
}

/// Whether the person can see Baton now: it is the app in front, and its window is on the screen
/// with some of it uncovered. A window that is hidden, minimized, waiting to hide or behind the
/// windows of another app is not seen, and what comes for the person then is told by a
/// notification (lib/src/arrival_notices.dart). Nobody sees a window of a Mac that is locked,
/// whose screen is off or that another account has in front, however the window is placed, so
/// that is checked first (`macIsNotInUse`): the notification waits for the person there.
private func personCanSeeBaton() -> Bool {
  if macIsNotInUse() { return false }
  guard NSApp.isActive, let window = (NSApp.delegate as? AppDelegate)?.mainFlutterWindow else {
    return false
  }
  return window.isVisible && !window.isMiniaturized && window.alphaValue > 0
    && window.occlusionState.contains(.visible)
}

/// Whether Baton opens when the person signs in, in the words of lib/src/always_on.dart. A Mac
/// older than macOS 13 has no way for an app to ask for that, so it says it cannot.
private func signInState() -> String {
  guard #available(macOS 13.0, *) else { return "unavailable" }
  return signInWords(SMAppService.mainApp.status)
}

@available(macOS 13.0, *)
private func signInWords(_ status: SMAppService.Status) -> String {
  switch status {
  case .enabled:
    return "on"
  case .requiresApproval:
    // Registered, and the person turned it off in System Settings, or has yet to say yes there.
    // Only the person can change that.
    return "needsApproval"
  case .notRegistered, .notFound:
    return "off"
  @unknown default:
    return "unavailable"
  }
}

/// Registers Baton to open at sign-in, or takes it back, and says where that stands. A refusal
/// from the system is thrown, for the app to say. What waits for the person in System Settings is
/// left alone: registering again changes nothing there.
private func setOpensAtSignIn(_ on: Bool) throws -> String {
  guard #available(macOS 13.0, *) else { return "unavailable" }
  let service = SMAppService.mainApp
  switch (on, service.status) {
  case (true, .notRegistered), (true, .notFound):
    try service.register()
  case (false, .enabled), (false, .requiresApproval):
    try service.unregister()
  default:
    break
  }
  return signInWords(service.status)
}

/// Opens the pane of System Settings where the person allows Baton to open at sign-in, and
/// nothing else.
private func openSignInSettings() -> Bool {
  guard #available(macOS 13.0, *) else { return false }
  SMAppService.openSystemSettingsLoginItems()
  return true
}

/// Gives what the person at this Mac is working on to a paired device that they allowed to ask: the
/// page that is in front in Safari or Chrome, and the text on the clipboard. It is read when the
/// question comes and kept nowhere. This also says where the person's consent to read the browsers
/// stands, and asks the system for it. The names of the channel and of the questions, and the words
/// of the answers, must be the ones in lib/src/continue_reader.dart.
private func answerAboutContinuing(messenger: FlutterBinaryMessenger) {
  browserWatch.start()
  let channel = FlutterMethodChannel(
    name: "io.github.jayunewal.baton/continuing", binaryMessenger: messenger)
  channel.setMethodCallHandler { call, result in
    switch call.method {
    case "read":
      readWhatThePersonIsWorkingOn(result)
    case "browser":
      guard let arguments = call.arguments as? [String: Any],
        let name = arguments["browser"] as? String,
        let browser = pageBrowsers.first(where: { $0.name == name })
      else {
        result(
          FlutterError(
            code: "bad_browser", message: "The browser was not Safari or Chrome.", details: nil))
        return
      }
      let ask = arguments["ask"] as? Bool ?? false
      // The system waits for the person to answer when it asks, so the main thread is not held.
      continueQueue.async {
        let answer = browserAccess(browser, ask: ask)
        DispatchQueue.main.async { result(answer) }
      }
    default:
      result(FlutterMethodNotImplemented)
    }
  }
}

/// The browsers that the page in front is read from.
private struct PageBrowser {
  /// The word for it in lib/src/continue_reader.dart.
  let name: String
  let bundleIdentifier: String
  /// What is asked of it, in AppleScript: nothing at all if it has no window, and otherwise the
  /// address and the title of the page in front, as a list. Three seconds is all that it gets.
  let script: String
}

private let pageBrowsers = [
  PageBrowser(
    name: "safari", bundleIdentifier: "com.apple.Safari",
    script: """
      with timeout of 3 seconds
        tell application id "com.apple.Safari"
          if (count of windows) is 0 then return {}
          set thePage to current tab of front window
          return {URL of thePage, name of thePage}
        end tell
      end timeout
      """),
  PageBrowser(
    // The page of a private window is never given: Chrome says which of its windows are.
    name: "chrome", bundleIdentifier: "com.google.Chrome",
    script: """
      with timeout of 3 seconds
        tell application id "com.google.Chrome"
          if (count of windows) is 0 then return {}
          set theWindow to front window
          if (mode of theWindow) is "incognito" then return {}
          set thePage to active tab of theWindow
          return {URL of thePage, title of thePage}
        end tell
      end timeout
      """),
]

/// A browser answers on a queue of its own, one question at a time: one that hangs holds nothing
/// else, and the system's own question to the person can wait here too.
private let continueQueue = DispatchQueue(
  label: "io.github.jayunewal.baton.continue", qos: .userInitiated)

/// What came of reading the page in front of a browser.
private enum PageRead {
  case page(url: String, title: String?)
  /// The browser has no window open, or has no page in it, or the window in front is private.
  case nothing
  /// The browser did not answer: it was not allowed to, or it took too long.
  case failed
}

/// What the person is working on, as the answer to the question "read" of
/// lib/src/continue_reader.dart: whichever of "pageUrl", "pageTitle" and "clipboard" there is, and
/// nothing at all for a Mac that nobody is using. An error says that the page could not be read and
/// that there is nothing else to give, which is not the same as there being nothing.
private func readWhatThePersonIsWorkingOn(_ result: @escaping FlutterResult) {
  // A Mac that is locked, or whose screen is off, gives nothing and not what it last held: the
  // person is not there to have chosen to hand it over.
  if macIsNotInUse() {
    result([String: Any]())
    return
  }
  var found = [String: Any]()
  if let text = clipboardText() {
    found["clipboard"] = text
  }
  guard let browser = browserWatch.inFront() else {
    result(found)
    return
  }
  continueQueue.async { [found] in
    let page = readPage(of: browser)
    DispatchQueue.main.async {
      var found = found
      switch page {
      case .page(let url, let title):
        found["pageUrl"] = url
        if let title = title {
          found["pageTitle"] = title
        }
        result(found)
      case .nothing:
        result(found)
      case .failed:
        // What was read of the clipboard is still given. With nothing else to give, the app is
        // told that it could not be read.
        if found.isEmpty {
          result(
            FlutterError(
              code: "unreadable", message: "The browser did not answer.", details: nil))
        } else {
          result(found)
        }
      }
    }
  }
}

/// Whether this Mac is locked, or its screen is off, or another account has the screen. The
/// dictionary of the session says the first and the last; a session that cannot be read is taken
/// as one that nobody is using.
private func macIsNotInUse() -> Bool {
  guard let session = CGSessionCopyCurrentDictionary() as? [String: Any] else { return true }
  if session["CGSSessionScreenIsLocked"] as? Bool == true { return true }
  if session["kCGSSessionOnConsoleKey"] as? Bool == false { return true }
  return CGDisplayIsAsleep(CGMainDisplayID()) != 0
}

/// The text on the clipboard. Nothing where there is none, where the app that copied it marked it
/// as secret or as one-time (a password manager does, by the convention of nspasteboard.org, so
/// that nothing keeps it), and where it is more than the protocol carries
/// (crates/baton-proto/src/limits.rs).
private func clipboardText() -> String? {
  let pasteboard = NSPasteboard.general
  let marks = ["org.nspasteboard.ConcealedType", "org.nspasteboard.TransientType"]
  if (pasteboard.types ?? []).contains(where: { marks.contains($0.rawValue) }) {
    return nil
  }
  guard let text = pasteboard.string(forType: .string),
    !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
    text.utf8.count <= 1024 * 1024
  else {
    return nil
  }
  return text
}

/// Which browser is the one to read: the one in front, and where the person is in another app, or
/// in Baton, the one that was in front last, if it is still open. A device that asks gets the page
/// that the person was on, and nothing from a browser that was left long ago and closed.
private final class BrowserWatch {
  private var last: PageBrowser?
  private var watching = false

  /// Starts to remember, from the browser that is in front now.
  func start() {
    guard !watching else { return }
    watching = true
    last = frontBrowser()
    NSWorkspace.shared.notificationCenter.addObserver(
      forName: NSWorkspace.didActivateApplicationNotification, object: nil, queue: .main
    ) { [weak self] note in
      let app = note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication
      if let browser = pageBrowser(withBundleIdentifier: app?.bundleIdentifier) {
        self?.last = browser
      }
    }
  }

  /// The browser to read, or nothing if there is none.
  func inFront() -> PageBrowser? {
    if let front = frontBrowser() {
      return front
    }
    guard let last = last,
      !NSRunningApplication.runningApplications(withBundleIdentifier: last.bundleIdentifier)
        .isEmpty
    else {
      return nil
    }
    return last
  }
}

private let browserWatch = BrowserWatch()

private func frontBrowser() -> PageBrowser? {
  pageBrowser(withBundleIdentifier: NSWorkspace.shared.frontmostApplication?.bundleIdentifier)
}

private func pageBrowser(withBundleIdentifier identifier: String?) -> PageBrowser? {
  guard let identifier = identifier else { return nil }
  return pageBrowsers.first(where: { $0.bundleIdentifier == identifier })
}

/// Reads the address and the title of the page in front of a browser. A browser that is not open is
/// not asked, because asking one by name would open it.
private func readPage(of browser: PageBrowser) -> PageRead {
  guard
    !NSRunningApplication.runningApplications(withBundleIdentifier: browser.bundleIdentifier)
      .isEmpty
  else {
    return .nothing
  }
  guard let script = NSAppleScript(source: browser.script) else { return .failed }
  var problem: NSDictionary?
  let answer = script.executeAndReturnError(&problem)
  if let problem = problem {
    // -1743 is a no from the person, and -1712 an answer that did not come in time. The number
    // says nothing of the page, so it is all that is written down.
    NSLog(
      "Baton could not read the page in front in \(browser.name): "
        + "\(problem["NSAppleScriptErrorNumber"] ?? "no number")")
    return .failed
  }
  // An empty list is a browser with no window open.
  guard answer.numberOfItems >= 1, let url = answer.atIndex(1)?.stringValue, !url.isEmpty else {
    return .nothing
  }
  return .page(url: url, title: answer.atIndex(2)?.stringValue)
}

/// Where the person's consent to read pages from a browser stands, in the words of
/// lib/src/continue_reader.dart. With `ask`, the system asks the person if it has not yet, which it
/// can do only while the browser is open, and this returns once they have answered.
private func browserAccess(_ browser: PageBrowser, ask: Bool) -> String {
  guard
    NSWorkspace.shared.urlForApplication(withBundleIdentifier: browser.bundleIdentifier) != nil
  else {
    return "unavailable"
  }
  guard
    !NSRunningApplication.runningApplications(withBundleIdentifier: browser.bundleIdentifier)
      .isEmpty
  else {
    return "notOpen"
  }
  let target = NSAppleEventDescriptor(bundleIdentifier: browser.bundleIdentifier)
  guard let address = target.aeDesc else { return "unavailable" }
  // '****' in four characters, which the system calls typeWildCard: any kind of event. The answer
  // is asked for in two calls so that the flag is a literal, whichever type the system's header
  // gives it.
  let anyEvent: UInt32 = 0x2A2A_2A2A
  let status =
    ask
    ? AEDeterminePermissionToAutomateTarget(address, anyEvent, anyEvent, true)
    : AEDeterminePermissionToAutomateTarget(address, anyEvent, anyEvent, false)
  switch status {
  case 0:
    return "allowed"
  case -1744:
    // errAEEventWouldRequireUserConsent: the system has not asked, and was told not to.
    return "needed"
  case -1743:
    // errAEEventNotPermitted: the person said no.
    return "denied"
  case -600:
    // procNotFound: the browser closed after it was found open.
    return "notOpen"
  default:
    return "unavailable"
  }
}
