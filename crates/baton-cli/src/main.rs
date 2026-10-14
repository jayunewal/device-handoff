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
mod words;

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

/// Warnings go to standard error. `-v` adds Baton's own progress and the network layer's
/// warnings, `-vv` also what the network layer is doing, and `BATON_LOG` takes a filter such
/// as `baton_core=debug` for anything more specific.
fn init_logging(verbose: u8) {
    let filter = EnvFilter::try_from_env("BATON_LOG")
        .unwrap_or_else(|_| EnvFilter::new(default_filter(verbose)));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
}

/// What is logged when `BATON_LOG` is not set.
fn default_filter(verbose: u8) -> &'static str {
    match verbose {
        // Only Baton's own warnings and errors. The network library logs errors of its own
        // as it shuts down, such as a relay connection that closed, and they mean nothing
        // to a person whose command worked. The command reports what went wrong itself.
        0 => "off,baton_core=warn,baton=warn",
        1 => "warn,baton_core=info,baton=info",
        2 => "info,baton_core=debug,baton=debug",
        _ => "trace",
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        sync::{Arc, Mutex},
    };

    use tracing_subscriber::fmt::MakeWriter;

    use super::*;

    /// A place for log lines to go that a test can read.
    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl MakeWriter<'_> for Captured {
        type Writer = Self;

        fn make_writer(&self) -> Self {
            self.clone()
        }
    }

    /// What the default filter for `verbose` lets through of a few typical log lines.
    fn logged(verbose: u8) -> String {
        let captured = Captured::default();
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new(default_filter(verbose)))
            .with_writer(captured.clone())
            .with_target(false)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::error!(target: "iroh::socket::transports::relay", "relay_recv_channel closed");
            tracing::info!(target: "iroh::endpoint", "network detail");
            tracing::warn!(target: "baton_core::inbox", "could not keep an item");
            tracing::info!(target: "baton_core::node", "listening");
            tracing::debug!(target: "baton_core::node", "a session opened");
        });
        let bytes = captured.0.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn by_default_only_batons_own_warnings_show() {
        let text = logged(0);
        assert!(text.contains("could not keep an item"), "{text}");
        assert!(!text.contains("relay_recv_channel"), "{text}");
        assert!(!text.contains("network detail"), "{text}");
        assert!(!text.contains("listening"), "{text}");
    }

    #[test]
    fn verbose_adds_progress_and_the_network_layers_errors() {
        let text = logged(1);
        assert!(text.contains("listening"), "{text}");
        assert!(text.contains("relay_recv_channel"), "{text}");
        assert!(!text.contains("a session opened"), "{text}");
    }

    #[test]
    fn more_verbose_shows_more() {
        assert!(logged(2).contains("a session opened"));
        assert!(logged(2).contains("network detail"));
        assert!(logged(3).contains("network detail"));
    }

    #[test]
    fn every_level_is_a_filter_the_logger_accepts() {
        for verbose in 0..=4 {
            assert!(
                EnvFilter::try_new(default_filter(verbose)).is_ok(),
                "level {verbose}"
            );
        }
    }
}
