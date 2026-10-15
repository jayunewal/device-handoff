# rust_builder

Glue that builds the Rust library of the Baton app (`crates/baton-ffi`) as part of the
iOS and macOS builds. It is a Flutter plugin only so that CocoaPods runs
[Cargokit](cargokit/README) at the right moment; there is no Dart code in it.

Android is added here when the Android app is.

`cargokit/` is copied from [Cargokit](https://github.com/irondash/cargokit) by Matej Knopp
(MIT, see `cargokit/LICENSE`) as the `flutter_rust_bridge` integration template ships it.
Do not edit it; update it by running the code generator's `integrate` command in a scratch
project and copying the folder.
