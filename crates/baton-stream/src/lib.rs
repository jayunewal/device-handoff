// SPDX-License-Identifier: Apache-2.0

//! Window streaming: the rules, without any input or output.
//!
//! A source (a Mac) sends a picture of one window to a viewer (an iPhone or an iPad) as a run
//! of encoded frames, one QUIC stream for each frame, and a frame that is late is reset and
//! replaced. The design is in `docs/adr/0009-window-streaming.md` and the wire format in
//! `docs/protocol.md`; the code here is the reference for both.
//!
//! This crate has no platform code, no network code and no clock of its own. What it will hold
//! is state machines that are told what happened and say what to do, so that they can be tested
//! with a simulated link in continuous integration. So far it holds the start of a stream:
//!
//! - [`header`] holds the first bytes of a stream and the header of a frame of video.

// This crate reads bytes that a paired device wrote. An index that is out of bounds panics, and
// a panic on input from the network is a remote crash, so the code spells out what happens when
// the data is short. Tests are exempt through `clippy.toml`.
#![warn(clippy::indexing_slicing)]

pub mod header;

pub use baton_proto::id::SessionId;

/// First byte of a unidirectional stream that carries one frame of video.
///
/// The other kinds are 1, a lane of a file transfer (ADR 0008), and [`KIND_CONTROL`].
pub const KIND_VIDEO: u8 = 2;

/// First byte of a unidirectional stream that carries control messages.
pub const KIND_CONTROL: u8 = 3;

// A receiver tells the kinds apart by this byte alone, and stops a stream whose kind it does not
// know (docs/protocol.md section 8). The kinds live in two crates, so the check that no two of
// them are the same is here, where all three can be seen, and fails to compile when a kind is
// added with a value that is taken.
const _: () = {
    assert!(baton_proto::transfer::lane::KIND_LANE != KIND_VIDEO);
    assert!(baton_proto::transfer::lane::KIND_LANE != KIND_CONTROL);
    assert!(KIND_VIDEO != KIND_CONTROL);
};
