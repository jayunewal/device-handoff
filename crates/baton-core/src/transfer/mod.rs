// SPDX-License-Identifier: Apache-2.0

//! Receiving and sending files: the engine behind the wire format of [`baton_proto::transfer`],
//! as `docs/adr/0008-file-transfer.md` decides it.
//!
//! The engine is built in pieces, each with tests of its own and each used by the next:
//!
//! - [`names`] chooses what everything in a received transfer is called.
//!
//! Nothing here is connected to the session yet, and `transfer.files` is not advertised.

pub mod names;
