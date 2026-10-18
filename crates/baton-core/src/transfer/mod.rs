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
//!   spans that come in any order, and checked against their hashes.
//!
//! Nothing here is connected to the session yet, and `transfer.files` is not advertised.

pub mod destination;
pub mod names;
pub mod staging;

#[cfg(test)]
mod testing;
