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
  }
}

/// Keeps the screen on while the app asks for it, which it does only while two devices pair. A
/// phone turns its screen off, and locks, after a while without a touch. Baton on a phone works
/// only while it is on the screen, so a screen that went off in the middle of pairing would take
/// the pairing with it. At any other time Baton leaves Auto-Lock as the person set it. The names
/// of the channel and of its methods must be the ones in lib/src/awake.dart.
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
    let support = try FileManager.default.url(
      for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
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
