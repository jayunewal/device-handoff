// SPDX-License-Identifier: Apache-2.0

//! The `baton` command-line program.
//!
//! It pairs this device with others, sends them links and text, and keeps what they send
//! in an inbox. `baton listen` is the long-running part: it receives, and takes the same
//! commands at a prompt.

// This crate is a program, not a library: its modules are private, so `pub` only marks what
// other modules of the program use.
#![allow(unreachable_pub)]

use std::{process::ExitCode, time::Duration};

use clap::Parser;
use tracing_subscriber::EnvFilter;

mod browser;
mod cli;
mod clipboard;
mod commands;
mod console;
mod context;
mod error;
mod select;
mod signal;
mod term;
mod util;

use cli::Cli;
use console::Console;
use context::Context;
use error::CliError;

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(cli.verbose);

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return report(&CliError::new(format!("could not start: {error}"))),
    };
    let code = runtime.block_on(run(cli));
    // A thread still waiting for a line from standard input must not hold the program up.
    runtime.shutdown_timeout(Duration::from_secs(2));
    code
}

async fn run(cli: Cli) -> ExitCode {
    let console = Console::stdin();
    let mut ctx = match Context::open(cli.home) {
        Ok(ctx) => ctx,
        Err(error) => return report(&error),
    };
    let result = commands::run(cli.command, &mut ctx, &console).await;
    ctx.shutdown().await;
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report(&error),
    }
}

fn report(error: &CliError) -> ExitCode {
    eprintln!("baton: {error}");
    error.exit_code()
}

/// Warnings go to standard error. `-v` adds Baton's own progress, `-vv` also what the
/// network layer is doing, and `BATON_LOG` takes a filter such as `baton_core=debug` for
/// anything more specific.
fn init_logging(verbose: u8) {
    let default = match verbose {
        0 => "error,baton_core=warn,baton=warn",
        1 => "warn,baton_core=info,baton=info",
        2 => "info,baton_core=debug,baton=debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_env("BATON_LOG").unwrap_or_else(|_| EnvFilter::new(default));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
}
