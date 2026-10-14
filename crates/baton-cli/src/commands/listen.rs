// SPDX-License-Identifier: Apache-2.0

//! `baton listen`: receive, and take commands at a prompt.

use std::io::Write as _;

use clap::Parser;
use tokio::sync::broadcast::error::RecvError;

use super::inbox::announcement;
use crate::{
    cli::Prompt, console::Console, context::Context, error::CliError, select, signal, term,
};

/// What the prompt looks like.
const PROMPT: &str = "baton> ";

/// Starts the node and keeps it running until Ctrl+C, a `quit`, or the end of input at
/// a terminal. Items that arrive are kept in the inbox and announced here.
pub async fn run(ctx: &mut Context, console: &Console) -> Result<(), CliError> {
    // Subscribe before the node starts, so nothing that arrives first is missed.
    let mut arrivals = ctx.inbox().subscribe();
    let node = ctx.node().await?;

    let id = node.device_id();
    println!(
        "Listening as \"{}\" ({}).",
        term::line(&ctx.config().device_name()),
        id.fingerprint()
    );
    let devices = ctx.devices()?;
    if devices.is_empty() {
        println!("No devices are paired yet: type `pair offer`, or `pair join <ticket>`.");
    } else {
        let names: Vec<String> = devices.iter().map(select::describe).collect();
        println!("Paired with: {}.", names.join(", "));
    }
    println!("What arrives is kept in the inbox (`baton inbox`, here or in another window).");
    println!("Type `help` for commands. Press Ctrl+C to stop.");

    let interactive = console.is_interactive();
    let mut lines = console.prompt();
    let mut typing = true;
    show_prompt(interactive);

    loop {
        tokio::select! {
            () = signal::interrupted() => break,
            arrival = arrivals.recv() => match arrival {
                Ok(entry) => {
                    println!("\n{}", announcement(&entry));
                    show_prompt(interactive);
                }
                Err(RecvError::Lagged(missed)) => {
                    println!("\n{missed} items arrived faster than they could be shown. `baton inbox` lists them.");
                    show_prompt(interactive);
                }
                Err(RecvError::Closed) => break,
            },
            line = lines.recv(), if typing => {
                if let Some(Some(line)) = line {
                    if !handle(&line, ctx, console).await {
                        break;
                    }
                    show_prompt(interactive);
                } else if interactive {
                    // The end of input at a terminal is Ctrl+D, which means quit.
                    println!();
                    break;
                } else {
                    // Anywhere else (a service, a pipe that was closed) there is nobody to
                    // type, and listening goes on.
                    typing = false;
                }
            }
        }
    }

    println!("Stopping...");
    Ok(())
}

/// Prints the prompt, if a person is there to read it.
pub(super) fn show_prompt(interactive: bool) {
    if interactive {
        print!("{PROMPT}");
        let _ = std::io::stdout().flush();
    }
}

/// Runs one line typed at the prompt. Returns `false` when it asks to stop.
async fn handle(line: &str, ctx: &mut Context, console: &Console) -> bool {
    let line = line.trim();
    if line.is_empty() {
        return true;
    }
    if ["quit", "exit", "q"]
        .iter()
        .any(|word| line.eq_ignore_ascii_case(word))
    {
        return false;
    }
    let Some(words) = crate::words::split(line) else {
        eprintln!("baton: a quote was opened and never closed.");
        return true;
    };
    match Prompt::try_parse_from(words) {
        Ok(prompt) => {
            if let Err(error) = super::run_in_session(prompt.command, ctx, console).await {
                eprintln!("baton: {error}");
            }
        }
        Err(error) => {
            // Usage mistakes and `help` come back as errors that know how to print
            // themselves. Printing one does not exit.
            let _ = error.print();
        }
    }
    true
}
