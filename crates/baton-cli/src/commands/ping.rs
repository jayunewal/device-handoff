// SPDX-License-Identifier: Apache-2.0

//! `baton ping`.

use crate::{
    cli::PingArgs,
    context::Context,
    error::{CliError, unreachable},
    select, term,
};

/// Checks that a paired device answers, and how long it takes.
pub async fn run(ctx: &mut Context, args: PingArgs) -> Result<(), CliError> {
    let devices = ctx.devices()?;
    let device = select::choose(&devices, args.device.as_deref())?;
    let node = ctx.node().await?;
    match node.ping(&device.id).await {
        Ok(report) => {
            println!(
                "{} answered in {} ms ({}).",
                term::line(device.display_name()),
                report.rtt.as_millis(),
                report.path.label()
            );
            Ok(())
        }
        Err(error) => Err(unreachable(error, &device)),
    }
}
