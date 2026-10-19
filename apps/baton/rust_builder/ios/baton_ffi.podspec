#
# Builds crates/baton-ffi with Cargokit before the pod compiles, and links the static
# library it makes into the app. The path in the script phase is relative to this folder,
# and Cargokit follows the symlinks Flutter puts around plugins, so it must stay a plain
# chain of "..".
#
Pod::Spec.new do |s|
  s.name             = 'baton_ffi'
  s.version          = '0.0.1'
  s.summary          = 'The Rust library of the Baton app.'
  s.description      = <<-DESC
Builds the Rust library of the Baton app and links it into the app.
                       DESC
  s.homepage         = 'https://github.com/jayunewal/device-handoff'
  s.license          = { :type => 'Apache-2.0' }
  s.author           = 'The Baton contributors'

  # The sources are the Swift that shows the windows of other devices (BatonViewer.swift,
  # BatonH264Decoder.swift and BatonStreamPlugin.swift) and the header of the C interface that it
  # calls the Rust library through (baton_stream_abi.h, which CocoaPods puts in the module, so
  # that the Swift sees it). The Rust library is linked into the framework that they make. The
  # files of ios/Classes and macos/Classes are the same, and test/viewer_files_test.dart fails if
  # they differ.
  s.source           = { :path => '.' }
  s.source_files     = 'Classes/**/*'
  s.dependency 'Flutter'
  s.platform = :ios, '15.0'
  s.swift_version = '5.0'

  s.script_phase = {
    :name => 'Build Rust library',
    # First argument is the relative path to the folder with Cargo.toml, second is the
    # name of the Rust library.
    :script => 'sh "$PODS_TARGET_SRCROOT/../cargokit/build_pod.sh" ../../../../crates/baton-ffi baton_ffi',
    :execution_position => :before_compile,
    :input_files => ['${BUILT_PRODUCTS_DIR}/cargokit_phony'],
    # Lets Xcode know that the static library named in -force_load below is made by this step.
    :output_files => ["${BUILT_PRODUCTS_DIR}/libbaton_ffi.a"],
  }
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    # Flutter.framework does not contain a i386 slice.
    'EXCLUDED_ARCHS[sdk=iphonesimulator*]' => 'i386',
    'OTHER_LDFLAGS' => '-force_load ${BUILT_PRODUCTS_DIR}/libbaton_ffi.a',
    # The app finds the library's functions by name, so the global symbols must survive when
    # Xcode strips a build.
    'STRIP_STYLE' => 'non-global',
  }

  # What the Rust library uses from the system, found by what its dependencies link:
  # SystemConfiguration to list the network interfaces, and Network to watch for changes in
  # them. A static library does not bring these with it, so the framework it is linked into
  # names them. When a new dependency needs another, the linker says "Undefined symbol".
  s.frameworks = 'Network', 'SystemConfiguration'
end
