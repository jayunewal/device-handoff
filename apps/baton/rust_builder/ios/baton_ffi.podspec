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

  # The sources are a single empty file. It makes CocoaPods build a framework, and the
  # Rust library is linked into that framework.
  s.source           = { :path => '.' }
  s.source_files     = 'Classes/**/*'
  s.dependency 'Flutter'
  s.platform = :ios, '13.0'
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
  }
end
