// SPDX-License-Identifier: Apache-2.0

//! Sending files, as `docs/adr/0008-file-transfer.md` decides it. It is built in pieces, each with
//! tests of its own and each used by the next:
//!
//! - [`selection`] walks the files and folders that a person chose into the list that the offer
//!   carries, before anything is offered.
//!
//! Nothing here is connected to a session yet.

pub mod selection;
