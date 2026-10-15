// SPDX-License-Identifier: Apache-2.0

//! What runs once, when the Dart side starts the library, and the log it leaves.

use flutter_rust_bridge::frb;
use tracing_subscriber::{fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _};

use crate::logbuf;

/// Sets up logging. Dart calls this by itself when the library loads.
///
/// Warnings and errors go to standard error, which Xcode and `flutter run` show, as do
/// Baton's own messages at `info` level. Set `RUST_LOG` to see more. The same lines are also
/// kept in memory for [`recent_log`], because on a phone, or in a Mac app opened from the
/// Finder, nobody sees standard error.
#[frb(init)]
pub fn init_app() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new("warn,baton_app=info,baton_core=info")
    });
    // A second start in the same process (a hot restart) finds it set up already.
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_ansi(false).with_writer(std::io::stderr))
        .with(fmt::layer().with_ansi(false).with_writer(logbuf::capture()))
        .try_init();
}

/// The last lines Baton logged since the library loaded, oldest first, as they were printed.
///
/// There are at most a few hundred, each cut to a limited length. Pairing codes are taken out
/// and so are characters that would act on a screen, so the lines are safe to show and to
/// paste into a bug report. They name this device and the ones paired with it, and they can
/// hold network addresses, so the person who copies them should read them first.
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
        assert!(
            lines.iter().any(
                |line| line.contains("a diagnostic line for the test") && line.contains("WARN")
            ),
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
    fn a_pairing_code_in_a_log_line_does_not_reach_the_log_the_app_shows() {
        init_app();

        tracing::warn!("could not join BATON1:AB12-cd34_EF56 just now");

        let lines = recent_log();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("could not join BATON1:[removed] just now")),
            "{lines:?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("AB12-cd34_EF56")),
            "{lines:?}"
        );
    }
}
