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
//! - `quic` is the wire over a node's connections. It is what connects the engine to the network:
//!   [`Node::send_files`](crate::Node::send_files) walks nothing and offers nothing itself, it starts
//!   the engine over this wire.
//!
//! The engine is tested against the real table of incoming transfers, in memory, and against a wire
//! that a test scripts, so that every fault that section 8 tells the sender about happens when the
//! test says. The wire over a node is tested between two real nodes on loopback.
//!
//! # Using it
//!
//! Walk what a person chose with [`Selection::walk`], tell them what it holds
//! ([`Selection::summary`]) and what was left out ([`Selection::left_out`]), and hand it to
//! [`Node::send_files`](crate::Node::send_files). What comes back is a [`Sending`]: watch its
//! [`progress`](Sending::progress), [`cancel`](Sending::cancel) it, and wait for its
//! [`outcome`](Sending::outcome). A transfer is given up when its [`Sending`] is dropped.

mod attempt;
mod book;
mod engine;
mod lane;
mod outcome;
mod policy;
mod produce;
mod progress;
mod quic;
pub mod selection;
mod source;
mod wire;

#[cfg(test)]
mod node_tests;
#[cfg(test)]
mod receiver_tests;
#[cfg(test)]
mod script_tests;
#[cfg(test)]
mod testing;

pub use engine::Sending;
pub use outcome::{Error, Outcome, Sent, Why};
pub use progress::{Progress, Stage};
pub use selection::Selection;

pub(crate) use engine::start;
pub(crate) use policy::Policy;
pub(crate) use quic::NodeWire;
pub(crate) use wire::Wire;
