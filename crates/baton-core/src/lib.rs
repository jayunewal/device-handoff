// SPDX-License-Identifier: Apache-2.0

//! The platform-neutral core of Baton.
//!
//! This crate owns everything a device needs to be a Baton device that is not
//! specific to an operating system: its [identity](identity::Identity), the
//! [list of devices it trusts](trust::TrustStore), per-device
//! [policy](policy::DevicePolicy), user [settings](config::Config), and the
//! on-disk layout that holds them ([`DataDir`]). Networking, pairing and
//! handoff build on these and arrive in later releases of the crate.
//!
//! The wire formats live in [`baton_proto`]; security reasoning for the pieces
//! here is in `docs/security.md` in the repository.
//!
//! State is stored in one directory per device identity. Open it with
//! [`DataDir::open`], take its [lock](DataDir::lock) while a process runs, then
//! load what you need:
//!
//! ```
//! use baton_core::{Config, DataDir, Identity, TrustStore};
//!
//! # fn main() -> baton_core::Result<()> {
//! # let scratch = tempfile::tempdir().unwrap();
//! let dir = DataDir::open(scratch.path().join("baton"))?;
//! let _lock = dir.lock()?;
//! let identity = Identity::load_or_create(&dir.identity_file())?;
//! let trust = TrustStore::load(dir.trust_file())?;
//! let config = Config::load(&dir.config_file())?;
//! println!("this device is {}", identity.device_id().fingerprint());
//! assert!(trust.is_empty());
//! assert!(config.network.lan_discovery);
//! # Ok(())
//! # }
//! ```

pub mod config;
pub mod error;
pub mod identity;
pub mod paths;
pub mod policy;
pub mod trust;

mod fsutil;

pub use config::Config;
pub use error::{Error, Result};
pub use identity::Identity;
pub use paths::{DataDir, DirLock, HOME_ENV};
pub use policy::{ClipboardPolicy, DevicePolicy, LinkPolicy};
pub use trust::{ContactHints, PairOutcome, PairedDevice, TrustEvent, TrustStore};
