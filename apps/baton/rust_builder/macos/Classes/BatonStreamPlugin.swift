// SPDX-License-Identifier: Apache-2.0

// The platform's viewer for a Flutter app on an iPhone, an iPad or a Mac: it shows the windows
// that other devices share. The Rust side of the app takes the pictures off the network
// (crates/baton-stream-abi), the decoder of the system makes them into pictures (VideoToolbox,
// BatonH264Decoder.swift), and Flutter draws them from a texture, which the Dart side shows with
// a Texture widget.
//
// On a Mac that has the picker of the system (macOS 14 and later) the plugin also registers the
// sharer, with which the person shares a window of their own: BatonCapture.swift picks and
// captures it, BatonH264Encoder.swift encodes it, and BatonSource.swift decides when a picture is
// made and hands it to the Rust side.

#if os(iOS)
import Flutter
#elseif os(macOS)
import FlutterMacOS
#endif
import CoreVideo
import Foundation

/// What Flutter draws a window from: the newest picture, which Flutter takes when it draws.
final class ScreenTexture: NSObject, FlutterTexture {
  private let lock = NSLock()
  private var picture: CVPixelBuffer?

  /// What is drawn when there is no picture. Flutter on an iPhone keeps drawing the last picture
  /// of a texture that answers nil, so the next window to use the texture would show the last
  /// picture of the one before until its own first came. A picture with nothing in it is drawn
  /// instead. It is made once, and every texture shares it.
  private static let blank = makeBlankPicture()

  /// Flutter asks on its raster thread, and owns what it is given.
  func copyPixelBuffer() -> Unmanaged<CVPixelBuffer>? {
    lock.lock()
    defer { lock.unlock() }
    guard let shown = picture ?? ScreenTexture.blank else { return nil }
    return Unmanaged.passRetained(shown)
  }

  func show(_ picture: CVPixelBuffer?) {
    lock.lock()
    self.picture = picture
    lock.unlock()
  }
}

/// A texture that a window is drawn from, with the number that Flutter gave it. The textures are
/// made by `ScreenPlaces` (BatonViewer.swift), which says when and why.
final class TextureSlot {
  let id: Int64
  private let texture: ScreenTexture
  private let registry: FlutterTextureRegistry

  init(id: Int64, texture: ScreenTexture, registry: FlutterTextureRegistry) {
    self.id = id
    self.texture = texture
    self.registry = registry
  }

  /// Shows a picture, or nothing. Any thread may call it.
  func show(_ picture: CVPixelBuffer?) {
    texture.show(picture)
    registry.textureFrameAvailable(id)
  }
}

/// How hot the device is, in the terms of the interface.
private func thermalLevel() -> UInt32 {
  switch ProcessInfo.processInfo.thermalState {
  case .nominal: return UInt32(BATON_THERMAL_NOMINAL)
  case .fair: return UInt32(BATON_THERMAL_FAIR)
  case .serious: return UInt32(BATON_THERMAL_SERIOUS)
  case .critical: return UInt32(BATON_THERMAL_CRITICAL)
  @unknown default: return UInt32(BATON_THERMAL_OTHER)
  }
}

/// What this device shows windows with: H.264 decoded by VideoToolbox, drawn from a texture.
final class DeviceViewer: ViewerPlatform {
  /// The largest picture and the highest rate the device is asked for. The hardware of every
  /// device that runs the app decodes H.264 at 4K, and a window that is larger is made smaller by
  /// the source, with its proportions kept.
  let limits = ViewerLimits(
    maxWidth: 3840, maxHeight: 2160, maxFps: 60, codecs: UInt32(BATON_CODEC_H264))

  private let places: ScreenPlaces<TextureSlot>

  init(places: ScreenPlaces<TextureSlot>) {
    self.places = places
  }

  func makeScreen(for window: WindowInfo, link: UInt64) throws -> MadeScreen {
    guard let slot = places.take() else {
      if places.status.made == 0 {
        // Flutter has taken no texture yet. The plugin goes on asking for them, and this asks once
        // more, now that a person wants a window, on the thread that Flutter needs.
        DispatchQueue.main.async { [places] in places.fill() }
        throw ScreenFailure(
          sentence: "This device is not ready to show windows yet. Try again in a moment.")
      }
      // A window is shown at a time, so this is the window before, which is not let go of yet.
      throw ScreenFailure(sentence: "The window before is still closing. Try again in a moment.")
    }
    let screen = WindowScreen(
      decoder: H264Decoder(),
      present: { slot.show($0) },
      failed: { _ = baton_stream_link_decode_failed(link) },
      thermal: thermalLevel,
      onRelease: { [places] in
        slot.show(nil)
        places.give(slot)
      })
    return MadeScreen(screen: screen, id: slot.id)
  }
}

/// The plugin of the app: it gives the Rust side the viewer, and on a Mac the sharer, once, when
/// Flutter registers it. This is before Dart runs, and so before the app starts and asks the Rust
/// side what it can show and share.
@objc(BatonStreamPlugin)
public class BatonStreamPlugin: NSObject, FlutterPlugin {
  /// The channel of the questions that the app asks the plugin. `controlAccess` says whether this
  /// device lets Baton steer the windows and screens it shares (`ControlAccess`), and
  /// `requestControlAccess` asks the system to let it. `screenAccess` says whether this device
  /// lets Baton record its screen with no picker (`ScreenAccess`), and `requestScreenAccess` asks
  /// the system to let it. `status` is for the tests of the app, and says what the plugin did
  /// (integration_test/app_test.dart). The names are the ones in
  /// apps/baton/lib/src/control_access.dart.
  static let channelName = "io.github.jayunewal.baton/stream"

  /// How many textures windows are drawn from, which is how many windows are shown at a time.
  private static let textureCount = 2

  /// How long to wait before each new try to get the textures, when Flutter did not give them at
  /// once. Flutter on a Mac takes a texture only once its engine runs, and its engine starts after
  /// the plugins are registered, within a second or so. A window that is hidden at launch may start
  /// it much later, which `DeviceViewer` takes care of when a window is offered.
  private static let delays: [TimeInterval] =
    [0.05, 0.1, 0.2, 0.4, 0.8, 1.6] + Array(repeating: 2.0, count: 12)

  /// The one plugin of the process; another engine of the same app shares it. Only the platform
  /// thread, which registers plugins, touches it.
  private static var shared: BatonStreamPlugin?

  /// What `status` says about the sharer on a device that registers none: an iPhone, an iPad, and
  /// a Mac before macOS 14, which has no picker of the system. No function of the interface
  /// returns a positive number.
  static let noSharer: Int32 = 1

  private let places: ScreenPlaces<TextureSlot>
  private let registration: Int32
  private let sharerRegistration: Int32

  public static func register(with registrar: FlutterPluginRegistrar) {
    #if os(iOS)
    let textures: FlutterTextureRegistry = registrar.textures()
    let messenger = registrar.messenger()
    #else
    let textures: FlutterTextureRegistry = registrar.textures
    let messenger = registrar.messenger
    #endif
    let plugin = shared ?? BatonStreamPlugin(textures: textures)
    shared = plugin
    registrar.addMethodCallDelegate(
      plugin, channel: FlutterMethodChannel(name: channelName, binaryMessenger: messenger))
  }

  private init(textures: FlutterTextureRegistry) {
    let places = ScreenPlaces<TextureSlot>(count: BatonStreamPlugin.textureCount) {
      let texture = ScreenTexture()
      let id = textures.register(texture)
      #if os(macOS)
      // Flutter on a Mac answers 0 when it could not take the texture: its ids are never 0.
      if id == 0 { return nil }
      #endif
      // Flutter on an iPhone takes every texture, and the first id it gives is 0.
      return TextureSlot(id: id, texture: texture, registry: textures)
    }
    self.places = places
    // The viewer is registered whether or not the textures are there: what the device can show
    // does not depend on them, and an app that has started does not hear of a viewer that is
    // registered after that. A viewer that was registered before is the one that stays.
    let code = registerViewer(DeviceViewer(places: places))
    registration = code
    if code != Int32(BATON_OK) && code != Int32(BATON_E_REGISTERED) {
      NSLog("Baton: the viewer was not registered (%d), so this device shows no windows.", code)
    }
    sharerRegistration = BatonStreamPlugin.registerDeviceSharer()
    super.init()

    keepTrying(
      after: BatonStreamPlugin.delays,
      later: { delay, work in
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: work)
      },
      attempt: { places.fill() },
      giveUp: {
        let status = places.status
        NSLog(
          "Baton: Flutter took %ld of %ld textures, so a window waits until one is offered.",
          status.made, status.wanted)
      })
  }

  /// Registers the sharer of this device, if it has one, and says what came of it.
  private static func registerDeviceSharer() -> Int32 {
    #if os(macOS)
    if #available(macOS 14.0, *) {
      let code = registerSharer(MacSharer())
      if code != Int32(BATON_OK) && code != Int32(BATON_E_REGISTERED) {
        NSLog("Baton: the sharer was not registered (%d), so this device shares no windows.", code)
      }
      return code
    }
    #endif
    return noSharer
  }

  public func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
    switch call.method {
    case "status":
      let status = places.status
      result([
        "viewer": Int(registration), "sharer": Int(sharerRegistration),
        "textures": status.made, "wanted": status.wanted, "tries": status.tries,
      ])
    case "controlAccess":
      result(controlAccess().rawValue)
    case "requestControlAccess":
      requestControlAccess()
      result(nil)
    case "screenAccess":
      result(screenAccess().rawValue)
    case "requestScreenAccess":
      requestScreenAccess()
      result(nil)
    default:
      result(FlutterMethodNotImplemented)
    }
  }

  /// Whether this device has a sharer: the one that was registered now, or before.
  private var hasSharer: Bool {
    sharerRegistration == Int32(BATON_OK) || sharerRegistration == Int32(BATON_E_REGISTERED)
  }

  /// Whether this device lets Baton hand input events to the windows of others and to the system.
  /// A device that registered no sharer shares no window.
  private func controlAccess() -> ControlAccess {
    #if os(macOS)
    return hasSharer ? macControlAccess() : .unavailable
    #else
    return .unavailable
    #endif
  }

  private func requestControlAccess() {
    #if os(macOS)
    requestMacControlAccess()
    #endif
  }

  /// Whether this device lets Baton record its screen with no picker, which a screen that a paired
  /// device asks for needs. A device that registered no sharer gives no screen.
  private func screenAccess() -> ScreenAccess {
    #if os(macOS)
    return hasSharer ? macScreenAccess() : .unavailable
    #else
    return .unavailable
    #endif
  }

  private func requestScreenAccess() {
    #if os(macOS)
    requestMacScreenAccess()
    #endif
  }
}
