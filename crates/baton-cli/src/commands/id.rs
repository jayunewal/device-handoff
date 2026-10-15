// SPDX-License-Identifier: Apache-2.0

//! `baton id`.

use crate::{context::Context, error::CliError, term};

/// Shows who this device is. The first time, that includes making its identity.
pub fn run(ctx: &mut Context) -> Result<(), CliError> {
    let id = ctx.device_id()?;
    let paired = ctx.devices()?.len();
    println!("Name:         {}", term::line(&ctx.config().device_name()));
    println!("Fingerprint:  {}", id.fingerprint());
    println!("Device id:    {id}");
    println!(
        "Paired with:  {paired} {}",
        if paired == 1 { "device" } else { "devices" }
    );
    println!("State folder: {}", ctx.dir().root().display());
    Ok(())
}
