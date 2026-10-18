// SPDX-License-Identifier: Apache-2.0

//! Receiving and sending files: the engine behind the wire format of [`baton_proto::transfer`],
//! as `docs/adr/0008-file-transfer.md` decides it.
//!
//! The engine is built in pieces, each with tests of its own and each used by the next:
//!
//! - [`names`] chooses what everything in a received transfer is called.
//! - [`destination`] is the folder that transfers are received into, and what its volume does
//!   with names.
//! - [`staging`] is one transfer that is being received: the files written at their place from
//!   spans that come in any order, and checked against their hashes. When the transfer has
//!   ended it puts what was checked in the destination, without replacing anything.
//! - [`receive`] applies the rules of the protocol for a receiver to what arrives on the lanes of
//!   one transfer: which lanes may be opened, which frames may come when, and what became of every
//!   file. It is built in pieces too, and so far has the bookkeeping, the reader of a lane, and
//!   the intake that applies the frames of the lanes to the staging folder.
//!
//! Nothing here is connected to the session yet, and `transfer.files` is not advertised.

pub mod destination;
pub mod names;
pub mod receive;
pub mod staging;

#[cfg(test)]
mod testing;
