# rust_builder

Glue that builds the Rust library of the Baton app (`crates/baton-ffi`) as part of the
iOS and macOS builds. It is a Flutter plugin only so that CocoaPods runs
[Cargokit](cargokit/README) at the right moment; there is no Dart code in it.

Android is added here when the Android app is.

`cargokit/` is copied from [Cargokit](https://github.com/irondash/cargokit) by Matej Knopp
(MIT, see `cargokit/LICENSE`) as the `flutter_rust_bridge` integration template ships it,
with one change. To update it, run the code generator's `integrate` command in a scratch
project, copy the folder and make the change again.

The change is in `getArtifactNames` in `cargokit/build_tool/lib/src/artifacts_provider.dart`:
the library name has each `-` turned into `_`, because Cargo names the library of the package
`baton-ffi` `libbaton_ffi.a`, and the template looks for `libbaton-ffi.a`. The crate in the
template is called `rust_lib_` followed by the name of the app, so it has no `-` to trip over.
`test/cargokit_test.dart` fails if the line is lost.
