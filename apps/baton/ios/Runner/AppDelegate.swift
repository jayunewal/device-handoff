import Flutter
import UIKit

@main
@objc class AppDelegate: FlutterAppDelegate, FlutterImplicitEngineDelegate {
  /// Kept here so that it lives as long as the app.
  private var screenKeeper: ScreenKeeper?

  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    keepBatonFolderOutOfBackups()
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  func didInitializeImplicitFlutterEngine(_ engineBridge: FlutterImplicitEngineBridge) {
    GeneratedPluginRegistrant.register(with: engineBridge.pluginRegistry)
    screenKeeper = ScreenKeeper(messenger: engineBridge.applicationRegistrar.messenger())
    answerWhereTheAppKeepsItsFiles(messenger: engineBridge.applicationRegistrar.messenger())
    showReceivedFilesInTheFilesApp(messenger: engineBridge.applicationRegistrar.messenger())
  }
}

/// Tells the app where its Library/Application Support is, which is where Baton makes its folder,
/// and where the files that other devices send are put. The app asks and does not look in its
/// environment: iOS 27 starts an app without HOME in it, and a Baton that looked for HOME did not
/// start. The names of the channel and of the questions must be the ones in
/// lib/src/support_folder.dart.
private func answerWhereTheAppKeepsItsFiles(messenger: FlutterBinaryMessenger) {
  let channel = FlutterMethodChannel(
    name: "io.github.jayunewal.baton/folder", binaryMessenger: messenger)
  channel.setMethodCallHandler { call, result in
    switch call.method {
    case "applicationSupport":
      do {
        let folder = try applicationSupportFolder().path
        result(folder)
      } catch {
        result(FlutterError(code: "no_folder", message: "\(error)", details: nil))
      }
    case "receiveFolder":
      do {
        let folder = try documentsFolder().path
        result(folder)
      } catch {
        result(FlutterError(code: "no_folder", message: "\(error)", details: nil))
      }
    default:
      result(FlutterMethodNotImplemented)
    }
  }
}

/// The folder that the system gives the app for the files that are not the person's documents.
private func applicationSupportFolder() throws -> URL {
  try FileManager.default.url(
    for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
}

/// The folder that the Files app shows as the app's own, under "On My iPhone" or "On My iPad", as
/// Info.plist says it may (UIFileSharingEnabled and LSSupportsOpeningDocumentsInPlace). The files
/// that other devices send are put there, so the person finds them.
private func documentsFolder() throws -> URL {
  try FileManager.default.url(
    for: .documentDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
}

/// Takes the person to the files that other devices sent: the Files app opens at the folder that
/// holds them, which is the Baton folder that it shows under "On My iPhone" or "On My iPad". The
/// Files app shows them and does not open them. The names of the channel and of the question must
/// be the ones in lib/src/file_shower.dart.
private func showReceivedFilesInTheFilesApp(messenger: FlutterBinaryMessenger) {
  let channel = FlutterMethodChannel(
    name: "io.github.jayunewal.baton/files", binaryMessenger: messenger)
  channel.setMethodCallHandler { call, result in
    switch call.method {
    case "show":
      guard let paths = call.arguments as? [String] else {
        result(FlutterError(code: "bad_paths", message: "The paths were not text.", details: nil))
        return
      }
      showInTheFilesApp(paths: paths, result: result)
    default:
      result(FlutterMethodNotImplemented)
    }
  }
}

/// Opens the Files app at the folder to show for `paths`, and says `false` when none of them is
/// there any more: the person may have moved or deleted them since. A folder that arrived alone is
/// opened, unless it is a package. Otherwise the folder that holds the first one is, which is the
/// folder they were all put in, and what arrived is in sight there. The Files app is told which
/// folder to open with the path of the folder under its own scheme, which is how it is asked to
/// open the Documents folder of an app.
private func showInTheFilesApp(paths: [String], result: @escaping FlutterResult) {
  let files = FileManager.default
  let there = paths.filter { files.fileExists(atPath: $0) }
  guard let first = there.first else {
    result(false)
    return
  }
  var isFolder: ObjCBool = false
  _ = files.fileExists(atPath: first, isDirectory: &isFolder)
  let item = URL(fileURLWithPath: first)
  // A folder with a name such as Report.pages is a document to the system, and the Files app could
  // open it as one. Like a file, it is shown by the folder that holds it.
  let isPackage = (try? item.resourceValues(forKeys: [.isPackageKey]))?.isPackage ?? false
  let alone = there.count == 1 && isFolder.boolValue && !isPackage
  let folder = alone ? item : item.deletingLastPathComponent()
  guard var address = URLComponents(url: folder, resolvingAgainstBaseURL: false) else {
    result(FlutterError(code: "no_address", message: "The folder has no address.", details: nil))
    return
  }
  address.scheme = "shareddocuments"
  guard let url = address.url else {
    result(FlutterError(code: "no_address", message: "The folder has no address.", details: nil))
    return
  }
  UIApplication.shared.open(url, options: [:]) { opened in
    if opened {
      result(true)
    } else {
      result(
        FlutterError(code: "not_opened", message: "The Files app did not open.", details: nil))
    }
  }
}

/// Keeps the screen on while the app asks for it, which it does while two devices pair and while
/// files go. A phone turns its screen off, and locks, after a while without a touch. Baton on a
/// phone works only while it is on the screen, so a screen that went off in the middle of pairing
/// or of a transfer would take it along. At any other time Baton leaves Auto-Lock as the person
/// set it. The names of the channel and of its methods must be the ones in lib/src/awake.dart.
private final class ScreenKeeper {
  /// Whether the app wants the screen kept on. It counts only while the app is on the screen.
  private var wanted = false
  private var observers: [NSObjectProtocol] = []

  init(messenger: FlutterBinaryMessenger) {
    let channel = FlutterMethodChannel(
      name: "io.github.jayunewal.baton/screen", binaryMessenger: messenger)
    channel.setMethodCallHandler { [weak self] call, result in
      switch call.method {
      case "keepOn":
        self?.want(true)
        result(nil)
      case "letTurnOff":
        self?.want(false)
        result(nil)
      case "isKeptOn":
        // What the system itself says, which is what a test of the app on a phone asks.
        result(UIApplication.shared.isIdleTimerDisabled)
      default:
        result(FlutterMethodNotImplemented)
      }
    }

    // The system may forget the setting while the app is away, so it is set again when the app
    // is back, if it is still wanted, and the app never holds the screen from the background.
    // The app and its scene each say when this happens, and either may be the one to say it.
    let center = NotificationCenter.default
    let back: [Notification.Name] = [
      UIApplication.didBecomeActiveNotification, UIScene.didActivateNotification,
    ]
    for name in back {
      observers.append(
        center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
          UIApplication.shared.isIdleTimerDisabled = self?.wanted ?? false
        })
    }
    let away: [Notification.Name] = [
      UIApplication.willResignActiveNotification, UIScene.willDeactivateNotification,
    ]
    for name in away {
      observers.append(
        center.addObserver(forName: name, object: nil, queue: .main) { _ in
          UIApplication.shared.isIdleTimerDisabled = false
        })
    }
  }

  deinit {
    for observer in observers {
      NotificationCenter.default.removeObserver(observer)
    }
  }

  private func want(_ on: Bool) {
    wanted = on
    UIApplication.shared.isIdleTimerDisabled = on
  }
}

/// Where Baton keeps its files, inside Library/Application Support. It holds the key that makes
/// this device what it is. The key must not leave the device, and a backup restored onto another
/// one would carry it there, so the folder is left out of backups. The name must be the one in
/// lib/src/rust_backend.dart.
private let batonFolderName = "baton-app"

private func keepBatonFolderOutOfBackups() {
  do {
    let support = try applicationSupportFolder()
    var folder = support.appendingPathComponent(batonFolderName, isDirectory: true)
    // Private to this app, as Baton makes it itself. An existing folder is left as it is.
    try FileManager.default.createDirectory(
      at: folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
    var values = URLResourceValues()
    values.isExcludedFromBackup = true
    try folder.setResourceValues(values)
  } catch {
    NSLog("Baton could not keep its folder out of backups: \(error)")
  }
}
