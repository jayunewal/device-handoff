// SPDX-License-Identifier: Apache-2.0

//! What each command does.

mod devices;
mod id;
mod inbox;
mod listen;
mod pair;
mod ping;
mod send;

use crate::{cli::Command, console::Console, context::Context, error::CliError};

/// How a command was started, which decides how it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Typed as `baton <command>`: do it, then exit.
    OneShot,
    /// Typed at the `baton listen` prompt: do it and go back to the prompt, which keeps
    /// the node running. A pairing offer is left waiting in the background.
    InSession,
}

/// Runs a command typed as `baton <command>`.
pub async fn run(command: Command, ctx: &mut Context, console: &Console) -> Result<(), CliError> {
    match command {
        Command::Listen => listen::run(ctx, console).await,
        other => run_other(other, ctx, console, Mode::OneShot).await,
    }
}

/// Runs a command typed at the `baton listen` prompt.
async fn run_in_session(
    command: Command,
    ctx: &mut Context,
    console: &Console,
) -> Result<(), CliError> {
    run_other(command, ctx, console, Mode::InSession).await
}

async fn run_other(
    command: Command,
    ctx: &mut Context,
    console: &Console,
    mode: Mode,
) -> Result<(), CliError> {
    match command {
        Command::Id => id::run(ctx),
        Command::Pair(crate::cli::PairCommand::Offer) => pair::offer(ctx, console, mode).await,
        Command::Pair(crate::cli::PairCommand::Join { ticket }) => {
            pair::join(ctx, console, mode, ticket).await
        }
        Command::Devices => devices::list(ctx),
        Command::Unpair(args) => devices::unpair(ctx, console, args).await,
        Command::Ping(args) => ping::run(ctx, args).await,
        Command::Send(command) => send::run(ctx, console, mode, command).await,
        Command::Inbox(args) => inbox::run(ctx, console, args).await,
        Command::Listen => Err(CliError::new("Already listening.")),
    }
}
