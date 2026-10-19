// SPDX-License-Identifier: Apache-2.0

//! The small C interface through which the platform code of a device (Swift on an Apple device)
//! shows the windows that other devices share. `include/baton_stream_abi.h` is the contract; this
//! crate is the Rust side of it, and holds every line of `unsafe` code that the interface needs.
//!
//! The Rust side takes the pictures off the network and puts them in order, and the platform
//! decodes and draws them (`docs/adr/0009-window-streaming.md` says why the pictures never pass
//! through the user interface). The two meet here:
//!
//! - The platform registers a vtable once, at launch ([`exports::baton_stream_register_viewer`]).
//!   [`registered_windows`] turns what it registered into the [`Windows`](baton_app::Windows) that
//!   the app is started with, and the app then shows a window by asking the platform for a screen.
//! - Each screen is a decoder that the session calls with its lock held. It converts the picture
//!   from the form of the wire (H.264 in Annex B form) into the form the Apple decoder takes
//!   (AVCC), holds the parameter sets to the size the device said it can show, and passes the
//!   result across the interface.
//! - A decoder that fails later, on a thread of its own, says so with a link that `open` gave it
//!   ([`exports::baton_stream_link_decode_failed`]).
//!
//! # Why this crate has `unsafe` code
//!
//! A C interface is made of pointers, and `CONTRIBUTING.md` asks for `unsafe` code to live in a
//! crate of its own, with the reason written down. This is that crate. `unsafe` is denied
//! everywhere in it except in two modules:
//!
//! - `platform` calls the platform. It reads the vtable that the platform registered, and holds
//!   the only calls of the functions in it. What it builds for a call (the structs of the header)
//!   it builds from Rust types and keeps alive until the call returns, and what the platform
//!   writes back (a size, a sentence, a pointer it will be given again) is checked before it is
//!   used. A screen is released exactly once.
//! - `exports` is the functions that the platform calls. Each checks its arguments, catches a
//!   panic, and answers with a number.
//!
//! A pointer is never kept past the call it was given in, in either direction, and a number
//! that stands for something the Rust side holds (the link of a screen) is looked up in a table
//! and not trusted: a platform that calls with a number that is not open is told so.
//!
//! # Keeping the functions in a library
//!
//! The functions of [`exports`] are `#[no_mangle]`. A library built from a crate that uses this
//! one, by a call or by nothing more than `use baton_stream_abi as _;`, has them, in its static
//! form and in its shared form, and needs no `pub use` for it. A crate that does not use this one
//! does not link it and has none of them. That was checked with a small library and a dependency
//! of its own, on Linux with Rust 1.99.0, in a debug build and in a release build with the thin
//! link-time optimization and the stripping that the workspace sets; it was not checked for an
//! Apple target, where the two Mac jobs of CI look for the functions in the app they build.
//! `baton-ffi` calls [`registered_windows`], and a test of the Dart package opens its shared
//! library and looks the functions up, so a change that makes the library stop carrying the
//! interface shows there.
//!
//! # What has not been run yet
//!
//! The tests play the platform in Rust, through the same C functions: they hold the layout to the
//! header, run a session against the registry, and check that nothing leaks and that a wrong call
//! is refused. No Swift code calls this yet, and a real Apple decoder has not been given a frame.

#[cfg(not(target_pointer_width = "64"))]
compile_error!(
    "the interface to the platform is for 64-bit targets, see include/baton_stream_abi.h"
);

pub mod abi;
#[allow(unsafe_code)]
pub mod exports;
#[cfg(test)]
mod fake;
#[cfg(test)]
mod fixture;
#[allow(unsafe_code)]
mod platform;
mod registry;
#[cfg(test)]
mod session_tests;
mod viewer;

pub use registry::{Error, Registry, registered_windows};
