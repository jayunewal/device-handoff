// SPDX-License-Identifier: Apache-2.0

// The platform's viewer for a Flutter app on an iPhone, an iPad or a Mac: it shows the windows
// that other devices share. The Rust side of the app takes the pictures off the network
// (crates/baton-stream-abi), the decoder of the system makes them into pictures (VideoToolbox,
// BatonH264Decoder.swift), and Flutter draws them from a texture, which the Dart side shows with
// a Texture widget.

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

  /// Flutter asks on its raster thread, and owns what it is given.
  func copyPixelBuffer() -> Unmanaged<CVPixelBuffer>? {
    lock.lock()
    defer { lock.unlock() }
    guard let picture else { return nil }
    return Unmanaged.passRetained(picture)
  }

  func show(_ picture: CVPixelBuffer?) {
    lock.lock()
    self.picture = picture
    lock.unlock()
  }
}

/// The textures that windows are drawn from. Flutter takes a texture only on the platform thread,
/// and the Rust side asks for a screen from a thread of its own and does not wait, so they are
/// registered when the app starts and a window takes one for as long as it is shown. A texture
/// that has nothing to show draws nothing and costs nothing.
final class TextureSlots {
  final class Slot {
    let id: Int64
    fileprivate let texture: ScreenTexture
    fileprivate let registry: FlutterTextureRegistry
    fileprivate var taken = false

    fileprivate init(id: Int64, texture: ScreenTexture, registry: FlutterTextureRegistry) {
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

  private let lock = NSLock()
  private let slots: [Slot]

  /// Registers `count` textures, on the platform thread. nil if Flutter took none.
  init?(registry: FlutterTextureRegistry, count: Int) {
    var made: [Slot] = []
    for _ in 0..<count {
      let texture = ScreenTexture()
      let id = registry.register(texture)
      // Flutter answers 0 when it could not take the texture.
      if id != 0 { made.append(Slot(id: id, texture: texture, registry: registry)) }
    }
    if made.isEmpty { return nil }
    slots = made
  }

  /// A texture that no window uses, or nil if every one is in use.
  func take() -> Slot? {
    lock.lock()
    defer { lock.unlock() }
    guard let free = slots.first(where: { !$0.taken }) else { return nil }
    free.taken = true
    return free
  }

  /// Gives the texture back, showing nothing.
  func give(_ slot: Slot) {
    slot.show(nil)
    lock.lock()
    slot.taken = false
    lock.unlock()
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

  private let slots: TextureSlots

  init(slots: TextureSlots) {
    self.slots = slots
  }

  func makeScreen(for window: WindowInfo, link: UInt64) throws -> MadeScreen {
    guard let slot = slots.take() else {
      // A window is shown at a time, so this is the window before, which is not let go of yet.
      throw ScreenFailure(sentence: "The window before is still closing. Try again in a moment.")
    }
    let screen = WindowScreen(
      decoder: H264Decoder(),
      present: { slot.show($0) },
      failed: { _ = baton_stream_link_decode_failed(link) },
      thermal: thermalLevel,
      onRelease: { [slots] in slots.give(slot) })
    return MadeScreen(screen: screen, id: slot.id)
  }
}

/// The plugin of the app: it gives the Rust side the viewer, once, when Flutter registers it. This
/// is before Dart runs, and so before the app starts and asks the Rust side what it can show.
@objc(BatonStreamPlugin)
public class BatonStreamPlugin: NSObject, FlutterPlugin {
  public static func register(with registrar: FlutterPluginRegistrar) {
    #if os(iOS)
    let textures: FlutterTextureRegistry = registrar.textures()
    #else
    let textures: FlutterTextureRegistry = registrar.textures
    #endif
    guard let slots = TextureSlots(registry: textures, count: 2) else {
      NSLog("Baton: Flutter took no texture, so this device shows no windows.")
      return
    }
    let code = registerViewer(DeviceViewer(slots: slots))
    // A viewer that was registered before is the one that stays (another engine of the same app).
    if code != Int32(BATON_OK) && code != Int32(BATON_E_REGISTERED) {
      NSLog("Baton: the viewer was not registered (%d), so this device shows no windows.", code)
    }
  }
}
