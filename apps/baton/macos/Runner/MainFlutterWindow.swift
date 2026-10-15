import Cocoa
import FlutterMacOS

class MainFlutterWindow: NSWindow {
  override func awakeFromNib() {
    keepBatonFolderOutOfBackups()

    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

    RegisterGeneratedPlugins(registry: flutterViewController)

    super.awakeFromNib()
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
