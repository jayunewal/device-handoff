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
//! - `wire` is what the sender needs from the network and nothing more: a connection to the
//!   receiver, the requests of section 8, and the lanes. `policy` holds the numbers the sender works
//!   to, and `progress` how far a transfer has got.
//! - `lane` writes one lane. `attempt` carries a transfer from the offer to the outcome on one
//!   connection, and `engine` makes attempts until the transfer is over.
//!
//! The engine is tested against the real table of incoming transfers, in memory, and against a wire
//! that a test scripts, so that every fault that section 8 tells the sender about happens when the
//! test says. Nothing here is connected to a node yet.

#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the sender is connected to a node by the change after this one"
    )
)]

mod attempt;
mod book;
mod engine;
mod lane;
mod outcome;
mod policy;
mod produce;
mod progress;
pub mod selection;
mod source;
mod wire;
