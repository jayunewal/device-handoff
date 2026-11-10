import AppIntents
import Flutter
import Foundation

// MARK: - What a shortcut hands to Baton, to send to a paired device

/// The links and texts that a shortcut handed to Baton and that the app has not shown yet, oldest
/// first. They live in memory only: nothing is written down, and what nobody took before the app
/// was closed is gone with it. The app takes them when it is on the screen, puts each in front of
/// the person, and sends it only when the person taps Send (lib/src/shares.dart).
final class HandedToBaton {
  static let shared = HandedToBaton()

  /// More than this are not kept, and the oldest go first: a shortcut that ran away is no reason
  /// to hold on to what it handed over.
  private static let most = 20

  private let lock = NSLock()
  private var texts: [String] = []

  func add(_ text: String) {
    lock.lock()
    defer { lock.unlock() }
    texts.append(text)
    if texts.count > HandedToBaton.most {
      texts.removeFirst(texts.count - HandedToBaton.most)
    }
  }

  /// What was handed over since the last call, oldest first. It is forgotten once it is given.
  func take() -> [String] {
    lock.lock()
    defer { lock.unlock() }
    let taken = texts
    texts = []
    return taken
  }
}

/// "Send with Baton", the action that Shortcuts lists for the app. A shortcut that has it, and
/// that is set to show in the share sheet, hands the link or the text that is shared to Baton. An
/// app extension is the usual way to be in the share sheet, but it needs an App Group to give what
/// it was handed to the app, and a free Apple ID cannot have one.
///
/// The app comes to the front and the person sends there. The action does not send by itself: it
/// does not know which device the person means, and it runs where Baton's own screens are not.
@available(iOS 16.0, *)
struct SendWithBatonIntent: AppIntent {
  static let title: LocalizedStringResource = "Send with Baton"
  static let description = IntentDescription(
    "Hands a link or some text to Baton, which asks which paired device to send it to.")
  static let openAppWhenRun = true

  @Parameter(title: "Link or text")
  var content: String

  func perform() async throws -> some IntentResult {
    HandedToBaton.shared.add(content)
    return .result()
  }
}

/// Gives the app what a shortcut handed over. The names of the channel and of the question must be
/// the ones in lib/src/shares.dart.
func answerAboutShares(messenger: FlutterBinaryMessenger) {
  let channel = FlutterMethodChannel(
    name: "io.github.jayunewal.baton/shares", binaryMessenger: messenger)
  channel.setMethodCallHandler { call, result in
    switch call.method {
    case "take":
      result(HandedToBaton.shared.take())
    default:
      result(FlutterMethodNotImplemented)
    }
  }
}
