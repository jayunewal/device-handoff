// SPDX-License-Identifier: Apache-2.0

//! `baton devices` and `baton unpair`.

use crate::{
    cli::UnpairArgs,
    console::Console,
    context::Context,
    error::CliError,
    select, term,
    util::{is_yes, unix_now},
};

/// Lists the paired devices.
pub fn list(ctx: &Context) -> Result<(), CliError> {
    let devices = ctx.devices()?;
    if devices.is_empty() {
        println!("No devices are paired yet.");
        println!(
            "Run `baton listen` on one device and type `pair offer` at its prompt, then run \
             `baton pair join <ticket>` on the other."
        );
        return Ok(());
    }

    let now = unix_now();
    let mut rows = vec![vec![
        "NAME".to_owned(),
        "SYSTEM".to_owned(),
        "FINGERPRINT".to_owned(),
        "LAST SEEN".to_owned(),
    ]];
    for device in &devices {
        rows.push(vec![
            term::line(device.display_name()),
            term::platform(device.platform).to_owned(),
            device.id.fingerprint().to_string(),
            device
                .last_seen_at
                .map_or_else(|| "never".to_owned(), |seen| term::ago(now, seen)),
        ]);
    }
    print!("{}", term::table(&rows));

    for device in devices.iter().filter(|device| device.name_changed) {
        println!(
            "\nNote: the device you know as \"{}\" now calls itself \"{}\". A name is only a \
             label, so go by the fingerprint.",
            term::line(device.display_name()),
            term::line(&device.name)
        );
    }
    Ok(())
}

/// Forgets a paired device.
pub async fn unpair(
    ctx: &mut Context,
    console: &Console,
    args: UnpairArgs,
) -> Result<(), CliError> {
    let devices = ctx.devices()?;
    let device = select::choose(&devices, Some(&args.device))?;
    let who = select::describe(&device);

    // Kept until the result is printed, so that a pairing request that arrived meanwhile is
    // put to the person after it, not between their answer and it.
    let mut question = if args.yes {
        None
    } else {
        Some(console.ask().await)
    };
    if let Some(question) = question.as_mut() {
        println!(
            "Unpair {who}? It will no longer be able to send you anything, and you will not be \
             able to send to it. Type yes to unpair."
        );
        if !is_yes(question.answer().await.as_deref()) {
            println!("Nothing changed.");
            return Ok(());
        }
    }

    let trust = ctx.trust_for_update()?;
    if trust.remove(&device.id)?.is_some() {
        println!("Unpaired {who}.");
    } else {
        println!("{who} was already unpaired.");
    }
    Ok(())
}
