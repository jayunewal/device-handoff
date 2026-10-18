// SPDX-License-Identifier: Apache-2.0

//! Sending files, as `docs/adr/0008-file-transfer.md` decides it. It is built in pieces, each with
//! tests of its own and each used by the next:
//!
//! - [`selection`] walks the files and folders that a person chose into the list that the offer
//!   carries, before anything is offered.
//! - `source` opens and reads one file, and checks it against its entry in the list each time.
//! - `book` keeps what is left to send of a transfer and what has become of each file, for lanes
//!   that read at the same time.
//! - `produce` reads the files into the frames that the lanes write.
//!
//! Nothing here is connected to a session yet.

mod book;
mod produce;
pub mod selection;
mod source;
