import Cocoa
import FlutterMacOS
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

    super.awakeFromNib()
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
