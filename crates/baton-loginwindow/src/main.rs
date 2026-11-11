// SPDX-License-Identifier: Apache-2.0

//! The `baton-loginwindow` program: Baton at the login window of a Mac
//! (`docs/adr/0019-baton-at-the-login-window.md`).
//!
//! launchd runs `baton-loginwindow serve` when the Mac shows its login window. The person runs
//! the other commands in Terminal, with `sudo`, to pair a phone with it, to see which devices are
//! paired and to forget one.

use std::{io::IsTerminal, path::PathBuf, process::ExitCode, time::Duration};

use baton_app::Windows;
use baton_loginwindow::{Program, Result, State, Terminal, devices, name_this_mac, pair, serve};
use baton_stream_abi::registered_windows;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

/// Baton at the login window of a Mac: a small Baton that starts with the Mac, shows the login
/// window to a phone that was paired with it, and passes what the person does on the phone.
#[derive(Debug, Parser)]
#[command(name = "baton-loginwindow", version)]
struct Cli {
    /// The folder that holds this program's own key and list of devices. It is not the folder of
    /// the Baton app, and the two share nothing.
    #[arg(long, global = true, value_name = "FOLDER")]
    state_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show the login window to the devices that are paired with this program, until it is
    /// stopped. launchd runs this.
    Serve,
    /// Pair a phone with this program: offer a ticket, and ask here for the six digits that the
    /// phone shows.
    Pair {
        /// The name of this Mac, which the phone shows with "(login window)" after it. It is
        /// kept; without it the name that was kept before stays, or the Mac's host name is used.
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
    },
    /// List the devices that are paired with this program.
    Devices,
    /// Take a device off the list, so that it cannot open the login window any more.
    Forget {
        /// The start of the device's id, as `devices` shows it: at least six characters.
        device: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging();

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("baton-loginwindow: could not start: {error}");
            return ExitCode::FAILURE;
        }
    };
    let result = runtime.block_on(run(cli));
    // A thread that still waits for a line from standard input must not hold the program up.
    runtime.shutdown_timeout(Duration::from_secs(2));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("baton-loginwindow: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    let state = State::open(cli.state_dir)?;
    match cli.command {
        Command::Serve => serve(&state, registered_windows(), interrupted()).await,
        Command::Pair { name } => {
            if let Some(name) = name {
                name_this_mac(&state, Some(&name))?;
            }
            // No platform is needed to pair: nothing is shown, and the folder is held only for
            // as long as the pairing takes.
            let program = Program::start_without_platform(&state, Windows::none()).await?;
            let result = pair(program.app(), &Terminal, interrupted()).await;
            program.stop().await;
            result.map(|_| ())
        }
        Command::Devices => {
            let listed = devices::list(&state)?;
            if listed.is_empty() {
                println!("No device is paired with the login window of this Mac.");
            }
            for device in &listed {
                println!("{}\n", device.describe());
            }
            Ok(())
        }
        Command::Forget { device } => {
            let gone = devices::forget(&state, &device)?;
            println!(
                "Forgotten: {}. It cannot open the login window of this Mac any more, and has to \
                 pair again.",
                gone.name
            );
            Ok(())
        }
    }
}

/// Ready when the program is told to stop: by the system (`SIGTERM`, which is how launchd stops
/// an agent), or by the person (`SIGINT`, from Control-C).
async fn interrupted() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        match (
            signal(SignalKind::terminate()),
            signal(SignalKind::interrupt()),
        ) {
            (Ok(mut terminate), Ok(mut interrupt)) => {
                tokio::select! {
                    _ = terminate.recv() => {}
                    _ = interrupt.recv() => {}
                }
            }
            (terminate, interrupt) => {
                // The signals keep their usual effect: they end the program, without the
                // goodbye to the devices that are connected.
                let failed = terminate.err().or(interrupt.err());
                tracing::warn!(?failed, "the program cannot catch the signals that stop it");
                std::future::pending::<()>().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Warnings, and the program's own account of what it does, go to standard error, where launchd
/// puts them in a file. `BATON_LOG` takes a filter for more.
fn init_logging() {
    let filter = EnvFilter::try_from_env("BATON_LOG").unwrap_or_else(|_| {
        EnvFilter::new("off,baton_loginwindow=info,baton_app=warn,baton_core=warn")
    });
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_target(false)
        .init();
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn the_state_folder_can_be_given_before_or_after_the_command() {
        let before =
            Cli::try_parse_from(["baton-loginwindow", "--state-dir", "/x", "devices"]).unwrap();
        let after =
            Cli::try_parse_from(["baton-loginwindow", "devices", "--state-dir", "/x"]).unwrap();
        assert_eq!(before.state_dir, after.state_dir);
        assert_eq!(before.state_dir, Some(PathBuf::from("/x")));
    }

    #[test]
    fn forget_needs_a_device() {
        assert!(Cli::try_parse_from(["baton-loginwindow", "forget"]).is_err());
        assert!(Cli::try_parse_from(["baton-loginwindow", "forget", "abcdef"]).is_ok());
    }
}
