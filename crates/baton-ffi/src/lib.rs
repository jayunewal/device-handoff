// SPDX-License-Identifier: Apache-2.0

//! The bridge between the Flutter app and `baton-app`.
//!
//! [`api`] is the part that is written by hand: Dart-friendly copies of the app's values and
//! one object, [`api::baton::Baton`], that wraps a running `baton_app::App`. The rest of the
//! crate is generated.
//!
//! Nothing here decides anything. A call goes straight to `baton_app`, and what comes back is
//! copied into the values Dart receives, so this crate cannot let a device do more than the
//! app layer does.
//!
//! The platform code of the app (Swift, on an iPhone and on a Mac) also calls functions of this
//! library by name, those of `include/baton_stream_abi.h` in `baton-stream-abi`: it registers
//! what it can show and what it can share, and it shares a window through them. The library
//! carries them, in its shared form as well as its static one, because
//! [`Baton::start`](api::baton::Baton::start) uses that crate.

pub mod api;
mod convert;
mod logbuf;
// The glue that `flutter_rust_bridge_codegen` writes. It is full of `unsafe` code, which is
// the only reason this crate is not under the workspace's `forbid(unsafe_code)`, and it is
// not written to the standard of the rest.
#[allow(
    unsafe_code,
    unreachable_pub,
    missing_docs,
    missing_debug_implementations,
    clippy::all,
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::dbg_macro
)]
mod frb_generated;
