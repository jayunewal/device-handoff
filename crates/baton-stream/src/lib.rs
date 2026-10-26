// SPDX-License-Identifier: Apache-2.0

//! Window streaming: the rules, without any input or output.
//!
//! A source (a Mac) sends a picture of one window to a viewer (an iPhone or an iPad) as a run
//! of encoded frames, one QUIC stream for each frame, and a frame that is late is reset and
//! replaced. The design is in `docs/adr/0009-window-streaming.md` and the wire format in
//! `docs/protocol.md`; the code here is the reference for both. When the screen of a source is
//! shared whole, the sound that it plays can go with it, in packets that travel the same way
//! (`docs/adr/0012-sound.md`).
//!
//! This crate has no platform code, no network code and no clock of its own. What it will hold
//! is state machines that are told what happened and say what to do, so that they can be tested
//! with a simulated link in continuous integration. So far it holds the start of a stream, the
//! source's side of the frames in flight, the viewer's side, and what a viewer that was granted
//! control may do and how it sends it:
//!
//! - [`header`] holds the first bytes of a stream and the header of a frame of video.
//! - [`sound`] holds the header of a packet of sound, the source's accounting of packets in
//!   flight, and the viewer's choice of which packet to play next.
//! - [`sender`] holds the source's accounting of frames in flight, and when to give up on one.
//! - [`viewer`] holds the viewer's choice of which frame to decode next, and its limits on the
//!   frame streams it reads and the pictures it holds.
//! - [`input`] holds the events that a viewer sends and the checks on their numbers.
//! - [`input_queue`] holds what a viewer does with the touches and keys of its person before it
//!   sends them: the limits on their rate and size, the clamping of positions to the picture, and
//!   the releases.
//! - [`gate`] holds the rules that confine a viewer's input to the shared window.
//! - [`control`] holds the messages on the two control streams of a session.
//! - [`h264`] turns an H.264 picture from the form it has on the wire into the form that the
//!   encoder and the decoder of a platform use, and back.

// This crate reads bytes that a paired device wrote. An index that is out of bounds panics, and
// a panic on input from the network is a remote crash, so the code spells out what happens when
// the data is short. Tests are exempt through `clippy.toml`.
#![warn(clippy::indexing_slicing)]

pub mod control;
pub mod gate;
pub mod h264;
pub mod header;
pub mod input;
pub mod input_queue;
pub mod sender;
pub mod sound;
pub mod viewer;

#[cfg(test)]
mod sim;

pub use baton_proto::id::SessionId;

/// First byte of a unidirectional stream that carries one frame of video.
///
/// The other kinds are 1, a lane of a file transfer (ADR 0008), [`KIND_CONTROL`] and
/// [`KIND_SOUND`].
pub const KIND_VIDEO: u8 = 2;

/// First byte of a unidirectional stream that carries control messages.
pub const KIND_CONTROL: u8 = 3;

/// First byte of a unidirectional stream that carries one packet of sound.
pub const KIND_SOUND: u8 = 4;

/// How many frame streams a viewer reads at once, and so how many a source keeps open or is
/// resetting: six in flight, and up to six more that were reset and that the viewer still counts
/// (ADR 0009).
pub const MAX_FRAME_STREAMS: usize = 12;

/// How many packet streams of sound a viewer reads at once, and so how many a source keeps open or
/// is resetting: eight in flight, and up to eight more that were reset and that the viewer still
/// counts (ADR 0012).
pub const MAX_SOUND_STREAMS: usize = 16;

// A receiver tells the kinds apart by this byte alone, and stops a stream whose kind it does not
// know (docs/protocol.md section 8). The kinds live in two crates, so the check that no two of
// them are the same is here, where all four can be seen, and fails to compile when a kind is
// added with a value that is taken.
const _: () = {
    assert!(baton_proto::transfer::lane::KIND_LANE != KIND_VIDEO);
    assert!(baton_proto::transfer::lane::KIND_LANE != KIND_CONTROL);
    assert!(baton_proto::transfer::lane::KIND_LANE != KIND_SOUND);
    assert!(KIND_VIDEO != KIND_CONTROL);
    assert!(KIND_VIDEO != KIND_SOUND);
    assert!(KIND_CONTROL != KIND_SOUND);
};
