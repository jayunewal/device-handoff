// SPDX-License-Identifier: Apache-2.0

//! What the Flutter app can call.
//!
//! Every public item in these modules becomes Dart code when `flutter_rust_bridge_codegen`
//! runs, so this is the whole surface between the two languages. It adds nothing to what
//! `baton_app` lets a device do.

pub mod baton;
pub mod error;
pub mod model;
pub mod startup;
