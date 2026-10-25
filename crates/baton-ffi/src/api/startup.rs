// SPDX-License-Identifier: Apache-2.0

//! What runs once, when the Dart side starts the library, and the log it leaves.

use std::sync::OnceLock;

use flutter_rust_bridge::frb;
use tracing_subscriber::{fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _};

use crate::logbuf;

/// Sets up logging. Dart calls this by itself when the library loads.
///
/// Warnings and errors go to standard error, which Xcode and `flutter run` show, as do
/// Baton's own messages at `info` level. Set `RUST_LOG` to see more. The same lines are also
/// kept in memory for [`recent_log`], because on a phone, or in a Mac app opened from the
/// Finder, nobody sees standard error. A panic is one of those lines too.
///
/// The setup is done once per process, so a second start (a hot restart of the app) changes
/// nothing. A call that comes while another is still doing it waits for that one, so whichever
/// call returns, a line logged or a panic raised after it is in the log.
#[frb(init)]
pub fn init_app() {
    static SET_UP: OnceLock<()> = OnceLock::new();
    SET_UP.get_or_init(|| {
        let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            tracing_subscriber::EnvFilter::new(
                "warn,baton_app=info,baton_core=info,baton_stream_abi=info",
            )
        });
        // Logging that something else has set up already is left as it is, and then so is the
        // way panics are shown.
        let first = tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().with_ansi(false).with_writer(std::io::stderr))
            .with(fmt::layer().with_ansi(false).with_writer(logbuf::capture()))
            .try_init()
            .is_ok();
        if first {
            logbuf::log_panics();
        }
    });
}

/// The last lines Baton logged since the library loaded, oldest first, as they were printed.
///
/// There are at most a few hundred, each cut to a limited length. They are meant to be pasted
/// into a bug report on a public page, so a line is made safe on the way in: a pairing code is
/// removed together with the rest of its event, a key is cut to the short form of a device id,
/// the account name in a path becomes `~`, a public IP address is shown as that and its port
/// (`public IPv4 (hidden):41641`), and characters that would act on a screen are replaced.
/// What is not taken out is still there: the names of this device and of the ones paired with
/// it, their short ids, the addresses of the local network and the address of the relay. The
/// person who copies the lines should read them first.
#[frb(sync)]
#[must_use]
pub fn recent_log() -> Vec<String> {
    logbuf::recent()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_logged_after_the_start_can_be_read_back() {
        init_app();

        tracing::warn!("a diagnostic line for the test");

        let lines = recent_log();
        // The name of the module stays whole: `::a` and `f::` are addresses to a parser.
        assert!(
            lines.iter().any(|line| line
                .ends_with("WARN baton_ffi::api::startup::tests: a diagnostic line for the test")),
            "{lines:?}"
        );
        // The same call starting again is not a second logger.
        init_app();
        tracing::warn!("after the second start");
        let after: Vec<_> = recent_log()
            .into_iter()
            .filter(|line| line.contains("after the second start"))
            .collect();
        assert_eq!(after.len(), 1, "{after:?}");
    }

    #[test]
    fn the_lines_of_the_thread_that_puts_events_to_a_window_are_in_the_log_the_app_shows() {
        init_app();

        tracing::info!(target: "baton_stream_abi::input", "a line of the window input for the test");
        tracing::info!(target: "iroh::endpoint", "a line of the network for the test");

        let lines = recent_log();
        assert!(
            lines.iter().any(|line| line.ends_with(
                "INFO baton_stream_abi::input: a line of the window input for the test"
            )),
            "{lines:?}"
        );
        // What the network library says at that level stays out of the log.
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("a line of the network for the test")),
            "{lines:?}"
        );
    }

    #[test]
    fn a_public_address_and_an_account_name_do_not_reach_the_log_the_app_shows() {
        init_app();

        tracing::warn!(
            "reached 9.9.9.9:41641 and 10.0.0.7:41641 from /Users/zoe/Library/baton-app"
        );

        let lines = recent_log();
        assert!(
            lines.iter().any(|line| line.contains(
                "reached public IPv4 (hidden):41641 and 10.0.0.7:41641 from /Users/~/Library/baton-app"
            )),
            "{lines:?}"
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("9.9.9.9") || line.contains("zoe")),
            "{lines:?}"
        );
    }

    #[test]
    fn a_panic_is_in_the_log_the_app_shows_as_safe_as_any_other_line() {
        init_app();

        let thread = std::thread::spawn(|| {
            panic!("the test panics, with BATON1:AB12-cd34 in /Users/zoe/Library/baton-app");
        });
        assert!(thread.join().is_err());

        let lines = recent_log();
        // The place is written with the separators of the system the test runs on.
        assert!(
            lines.iter().any(|line| line
                .contains(" ERROR baton_ffi::logbuf::panics: panicked at crates")
                && line.contains("startup.rs:")
                && line.ends_with("the test panics, with BATON1:[removed]")),
            "{lines:?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("AB12-cd34")),
            "{lines:?}"
        );
    }

    #[test]
    fn a_panic_with_an_account_name_in_it_loses_the_name() {
        init_app();

        let thread = std::thread::spawn(|| {
            let path = "/Users/zoe/Library/baton-app";
            panic!("could not open {path}");
        });
        assert!(thread.join().is_err());

        let lines = recent_log();
        assert!(
            lines
                .iter()
                .any(|line| line.ends_with("could not open /Users/~/Library/baton-app")),
            "{lines:?}"
        );
        assert!(!lines.iter().any(|line| line.contains("zoe")), "{lines:?}");
    }

    #[test]
    fn a_pairing_code_in_a_log_line_does_not_reach_the_log_the_app_shows() {
        init_app();

        tracing::warn!("could not join BATON1:AB12-cd34_EF56 just now");

        let lines = recent_log();
        assert!(
            lines
                .iter()
                .any(|line| line.ends_with("could not join BATON1:[removed]")),
            "{lines:?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("AB12-cd34_EF56")),
            "{lines:?}"
        );
    }
}
