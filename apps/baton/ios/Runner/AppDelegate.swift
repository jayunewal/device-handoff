import AVFoundation
import Flutter
import LocalAuthentication
import Security
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
    keepMacPasswords(messenger: engineBridge.applicationRegistrar.messenger())
    checkTheOwner(messenger: engineBridge.applicationRegistrar.messenger())
    answerAboutPermissions(messenger: engineBridge.applicationRegistrar.messenger())
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

// MARK: - Passwords of Macs to unlock after a restart

/// Keeps what the app needs to unlock a Mac that FileVault holds locked after a restart
/// (lib/src/mac_passwords.dart): for each Mac, a record with its address, the account and the
/// host keys that are trusted, and the account's password. The names of the channel and of its
/// methods must be the ones in that file.
///
/// Both are kept in the keychain, on this device only: in no backup, and never moved to another
/// device. The record can be read whenever the device is unlocked, so that the app can list the
/// Macs. The password can be read only with a face that the system recognises (Face ID, or Touch
/// ID on a device without it). The passcode does not stand in for the face, and the password stops
/// opening when a face is added or removed (`biometryCurrentSet`), so that someone who knows the
/// passcode cannot add their own face and read it. It goes when the passcode is removed.
///
/// Nothing here writes a password, a record or an address to a log.
private func keepMacPasswords(messenger: FlutterBinaryMessenger) {
  let channel = FlutterMethodChannel(
    name: "io.github.jayunewal.baton/mac-passwords", binaryMessenger: messenger)
  channel.setMethodCallHandler { call, result in
    let arguments = call.arguments as? [String: Any] ?? [:]
    switch call.method {
    case "list":
      result(MacPasswords.records())
    case "save":
      guard let id = arguments["id"] as? String, let record = arguments["record"] as? String
      else {
        result(FlutterError(code: "bad_arguments", message: nil, details: nil))
        return
      }
      result(MacPasswords.save(id: id, record: record, password: arguments["password"] as? String))
    case "open":
      guard let id = arguments["id"] as? String, let reason = arguments["reason"] as? String else {
        result(FlutterError(code: "bad_arguments", message: nil, details: nil))
        return
      }
      // Reading the password shows the prompt for the face and waits for it, which must not hold
      // the main thread.
      DispatchQueue.global(qos: .userInitiated).async {
        let opened = MacPasswords.open(id: id, reason: reason)
        DispatchQueue.main.async { result(opened) }
      }
    case "forget":
      guard let id = arguments["id"] as? String else {
        result(FlutterError(code: "bad_arguments", message: nil, details: nil))
        return
      }
      MacPasswords.forget(id: id)
      result(nil)
    default:
      result(FlutterMethodNotImplemented)
    }
  }
}

private enum MacPasswords {
  static let recordService = "io.github.jayunewal.baton.mac-record"
  static let passwordService = "io.github.jayunewal.baton.mac-password"

  /// Every record, as the text the app saved.
  static func records() -> Any {
    let query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: recordService,
      kSecMatchLimit as String: kSecMatchLimitAll,
      kSecReturnData as String: true,
    ]
    var found: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &found)
    if status == errSecItemNotFound { return [String]() }
    guard status == errSecSuccess, let items = found as? [Data] else {
      return failure("keychain", status)
    }
    return items.compactMap { String(data: $0, encoding: .utf8) }
  }

  /// Saves the record, and the password when there is one. Without one, the password that is
  /// kept stays as it is.
  static func save(id: String, record: String, password: String?) -> Any? {
    if let password {
      // A password that needs a face can be kept only on a device with a passcode and a face
      // that is set up. Asking first gives the app a reason it can say, where the keychain would
      // give a number that says little.
      var problem: NSError?
      if !LAContext().canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &problem)
      {
        return FlutterError(
          code: "no_face", message: problem.map { "\($0.domain) \($0.code)" }, details: nil)
      }
      guard
        let rule = SecAccessControlCreateWithFlags(
          nil, kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly, .biometryCurrentSet, nil)
      else {
        return FlutterError(code: "keychain", message: "access rule", details: nil)
      }
      delete(service: passwordService, id: id)
      let add: [String: Any] = [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: passwordService,
        kSecAttrAccount as String: id,
        kSecAttrAccessControl as String: rule,
        kSecValueData as String: Data(password.utf8),
      ]
      let status = SecItemAdd(add as CFDictionary, nil)
      if status != errSecSuccess { return failure("keychain", status) }
    }
    delete(service: recordService, id: id)
    let add: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: recordService,
      kSecAttrAccount as String: id,
      kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
      kSecValueData as String: Data(record.utf8),
    ]
    let status = SecItemAdd(add as CFDictionary, nil)
    return status == errSecSuccess ? nil : failure("keychain", status)
  }

  /// The password, after the person has shown their face. The prompt says `reason`.
  static func open(id: String, reason: String) -> Any {
    let context = LAContext()
    context.localizedReason = reason
    // The passcode is no way round the face: the password's rule has none, and the prompt does
    // not offer it.
    context.localizedFallbackTitle = ""
    let query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: passwordService,
      kSecAttrAccount as String: id,
      kSecMatchLimit as String: kSecMatchLimitOne,
      kSecReturnData as String: true,
      kSecUseAuthenticationContext as String: context,
    ]
    var found: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &found)
    switch status {
    case errSecSuccess:
      guard let data = found as? Data, let password = String(data: data, encoding: .utf8) else {
        return FlutterError(code: "keychain", message: "not text", details: nil)
      }
      return password
    case errSecUserCanceled:
      return FlutterError(code: "cancelled", message: nil, details: nil)
    case errSecItemNotFound:
      // Also what a password whose faces have changed looks like: the system took it away.
      return FlutterError(code: "gone", message: nil, details: nil)
    case errSecAuthFailed:
      return FlutterError(code: "not_recognised", message: nil, details: nil)
    default:
      return failure("keychain", status)
    }
  }

  static func forget(id: String) {
    delete(service: passwordService, id: id)
    delete(service: recordService, id: id)
  }

  private static func delete(service: String, id: String) {
    let query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service,
      kSecAttrAccount as String: id,
    ]
    SecItemDelete(query as CFDictionary)
  }

  private static func failure(_ code: String, _ status: OSStatus) -> FlutterError {
    NSLog("Baton: Mac passwords: keychain status %d", status)
    return FlutterError(code: code, message: "status \(status)", details: nil)
  }
}

// MARK: - The person at this device

/// Asks the person who holds this device to show that it is them, before the app asks another
/// device for its screen (lib/src/owner_check.dart): their face (Face ID, or Touch ID on a device
/// without it) and, when the face is not recognised, the passcode of this device. The names of the
/// channel and of the question, and the codes of the failures, must be the ones in that file.
///
/// Each question is asked afresh, on a context of its own, so that a face that was recognised a
/// moment ago does not stand for this one. Whatever is not a clear yes is an error, and the app
/// does not go on without a yes. Nothing here writes the reason to a log.
private func checkTheOwner(messenger: FlutterBinaryMessenger) {
  let channel = FlutterMethodChannel(
    name: "io.github.jayunewal.baton/owner", binaryMessenger: messenger)
  channel.setMethodCallHandler { call, result in
    switch call.method {
    case "ask":
      // The system's question fails for a reason that is empty, so none is let through.
      guard let reason = call.arguments as? String, !reason.isEmpty else {
        result(FlutterError(code: "bad_arguments", message: nil, details: nil))
        return
      }
      OwnerCheck.ask(reason: reason) { answer in result(answer) }
    default:
      result(FlutterMethodNotImplemented)
    }
  }
}

private enum OwnerCheck {
  /// Shows the question, and calls `done` on the main thread with `true` when the person was
  /// confirmed and with the error to give back when they were not.
  static func ask(reason: String, done: @escaping (Any) -> Void) {
    let context = LAContext()
    // A face is asked for every time. This is also the default.
    context.touchIDAuthenticationAllowableReuseDuration = 0
    // The passcode is the way round a face that is not recognised, so a device that has one can
    // always be asked. One that has none cannot, and the system says so here and not in the
    // question.
    var problem: NSError?
    guard context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &problem) else {
      done(refusal(problem))
      return
    }
    context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason) {
      confirmed, error in
      // The system answers on a queue of its own, and Flutter is answered on the main one.
      DispatchQueue.main.async {
        if confirmed {
          done(true)
        } else {
          done(refusal(error))
        }
      }
    }
  }

  /// The error to give back for a question that did not end in a yes.
  private static func refusal(_ error: Error?) -> FlutterError {
    switch (error as? LAError)?.code {
    case .userCancel, .systemCancel, .appCancel:
      return FlutterError(code: "cancelled", message: nil, details: nil)
    case .authenticationFailed:
      return FlutterError(code: "not_recognised", message: nil, details: nil)
    case .passcodeNotSet:
      return FlutterError(code: "no_lock", message: nil, details: nil)
    default:
      NSLog("Baton: owner check: LocalAuthentication code %ld", (error as NSError?)?.code ?? 0)
      return FlutterError(code: "failed", message: nil, details: nil)
    }
  }
}

/// Says where the camera and Face ID stand, asks for the camera, and opens Baton's page in
/// Settings, where each permission it has is turned on. The names of the channel, of the questions
/// and of the permissions, and the words of the answers, must be the ones in
/// lib/src/permissions.dart.
private func answerAboutPermissions(messenger: FlutterBinaryMessenger) {
  let channel = FlutterMethodChannel(
    name: "io.github.jayunewal.baton/permissions", binaryMessenger: messenger)
  channel.setMethodCallHandler { call, result in
    let permission = call.arguments as? String ?? ""
    switch call.method {
    case "state":
      result(permissionState(permission))
    case "request":
      guard permission == "camera" else {
        // Face ID is asked for the first time it is used, to open or unlock a Mac.
        result(permissionState(permission))
        return
      }
      AVCaptureDevice.requestAccess(for: .video) { _ in
        DispatchQueue.main.async { result(permissionState(permission)) }
      }
    case "openSettings":
      guard let url = URL(string: UIApplication.openSettingsURLString) else {
        result(false)
        return
      }
      UIApplication.shared.open(url) { opened in result(opened) }
    default:
      result(FlutterMethodNotImplemented)
    }
  }
}

/// Where a permission stands, in the words of lib/src/permissions.dart, or nothing for one that
/// this device cannot have.
private func permissionState(_ permission: String) -> String? {
  switch permission {
  case "camera":
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
  case "faceId":
    return faceIdState()
  default:
    return nil
  }
}

/// What this device says about letting Baton use Face ID. A device with no Face ID answers nothing.
/// The system does not say whether it has asked the person yet, so a Face ID that Baton may use is
/// "allowed", and so is one that is locked for now after failed tries, which the passcode frees.
private func faceIdState() -> String? {
  let context = LAContext()
  var error: NSError?
  let usable = context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error)
  guard context.biometryType == .faceID else { return nil }
  if usable { return "allowed" }
  switch (error as? LAError)?.code {
  case .biometryLockout:
    return "allowed"
  case .biometryNotEnrolled, .passcodeNotSet:
    return "notSetUp"
  case .biometryNotAvailable:
    return "denied"
  default:
    return nil
  }
}
