# rust_builder

Glue that builds the Rust library of the Baton app (`crates/baton-ffi`) as part of the
iOS and macOS builds. It is a Flutter plugin only so that CocoaPods runs
[Cargokit](cargokit/README) at the right moment; there is no Dart code in it.

Android is added here when the Android app is.

`cargokit/` is copied from [Cargokit](https://github.com/irondash/cargokit) by Matej Knopp
(MIT, see `cargokit/LICENSE`) as the `flutter_rust_bridge` integration template ships it,
with two small changes. To update it, run the code generator's `integrate` command in a scratch
project, copy the folder and make the change again.

The changes are one line each, in `cargokit/build_tool/lib/src`: `getArtifactNames` in
`artifacts_provider.dart`, and `libName` in `build_pod.dart`. Both turn each `-` of the package
name into `_`, because Cargo names the library of the package `baton-ffi` `libbaton_ffi.a`, and
the template looks for, and writes, `libbaton-ffi.a`. The crate in the template is called
`rust_lib_` followed by the name of the app, so it has no `-` to trip over.
`test/cargokit_test.dart` fails if either line is lost.
