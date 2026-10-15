// SPDX-License-Identifier: Apache-2.0

//! Settings a screen asks for before the app starts.
//!
//! A computer can tell other devices its own name, but a phone usually cannot (it answers
//! `localhost`, or a generic "iPhone"), and a name shown at pairing is kept: a device is
//! shown under the name it had when it paired. So a screen asks the person for one first
//! and saves it here, and only then starts the [`App`](crate::App).

use std::path::Path;

use baton_core::{Config, DataDir};

use crate::{Error, Result};

/// The name saved for this device in `data_dir`, if the person has chosen one.
///
/// `None` means none has been chosen yet; the app then falls back to the computer's own
/// name, which on a phone is rarely a good one.
pub fn device_name(data_dir: &Path) -> Result<Option<String>> {
    let dir = DataDir::open(data_dir).map_err(|e| Error::local(&e))?;
    let config = Config::load(&dir.config_file()).map_err(|e| Error::local(&e))?;
    Ok(config.device_name)
}

/// Saves `name` as the name this device shows other devices. It takes effect the next time
/// the app starts, and only for devices that pair after that.
///
/// Every other setting in the settings file stays as it is.
///
/// # Errors
///
/// [`ErrorKind::BadName`](crate::ErrorKind::BadName) for a name with nothing visible in it,
/// whose message the screen that asked for the name can show as it is.
pub fn set_device_name(data_dir: &Path, name: &str) -> Result<()> {
    if !Config::is_usable_device_name(name) {
        return Err(Error::bad_name());
    }
    let dir = DataDir::open(data_dir).map_err(|e| Error::local(&e))?;
    Config::save_device_name(&dir.config_file(), name).map_err(|e| Error::local(&e))
}
