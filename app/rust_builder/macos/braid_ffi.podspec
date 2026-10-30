#
# To learn more about a Podspec see http://guides.cocoapods.org/syntax/podspec.html.
# Run `pod lib lint braid_ffi.podspec` to validate before publishing.
#
Pod::Spec.new do |s|
  s.name             = 'braid_ffi'
  s.version          = '0.0.1'
  s.summary          = 'Мост Flutter ↔ ядро Braid (flutter_rust_bridge + cargokit).'
  s.description      = <<-DESC
Мост Flutter ↔ ядро Braid: Rust собирается cargokit-ом внутри flutter build.
                       DESC
  s.homepage         = 'https://github.com/alextixru/braid'
  s.license          = { :type => 'См. THIRD_PARTY.md' }
  s.author           = { 'Braid' => 'noreply@braid.invalid' }

  # This will ensure the source files in Classes/ are included in the native
  # builds of apps using this FFI plugin. Podspec does not support relative
  # paths, so Classes contains a forwarder C file that relatively imports
  # `../src/*` so that the C sources can be shared among all target platforms.
  s.source           = { :path => '.' }
  s.source_files     = 'Classes/**/*'
  s.dependency 'FlutterMacOS'

  s.platform = :osx, '12.0'
  s.pod_target_xcconfig = { 'DEFINES_MODULE' => 'YES' }
  s.swift_version = '5.0'

  s.script_phase = {
    :name => 'Build Rust library',
    # First argument is relative path to the `rust` folder, second is name of rust library
    :script => 'sh "$PODS_TARGET_SRCROOT/../cargokit/build_pod.sh" ../../rust braid_ffi',
    :execution_position => :before_compile,
    :input_files => ['${BUILT_PRODUCTS_DIR}/cargokit_phony'],
    # Let XCode know that the static library referenced in -force_load below is
    # created by this build step.
    :output_files => ["${BUILT_PRODUCTS_DIR}/libbraid_ffi.a"],
  }
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    # Flutter.framework does not contain a i386 slice.
    'EXCLUDED_ARCHS[sdk=iphonesimulator*]' => 'i386',
    # Статическая библиотека не несёт директив линковки из build.rs — список из
    # `cargo rustc --crate-type staticlib -- --print native-static-libs`.
    # libawg.dylib встраивается в .app фазой «Embed braid-helper and libawg».
    'OTHER_LDFLAGS' => '-force_load ${BUILT_PRODUCTS_DIR}/libbraid_ffi.a ' \
      '-L"${PODS_TARGET_SRCROOT}/../../../engines/awg-go/dist/darwin" -lawg ' \
      '-framework SystemConfiguration -framework Security -framework CoreFoundation -liconv',
  }
end