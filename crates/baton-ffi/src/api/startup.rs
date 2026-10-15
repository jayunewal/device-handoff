// SPDX-License-Identifier: Apache-2.0

//! What runs once, when the Dart side starts the library.

use flutter_rust_bridge::frb;

/// Sets up logging. Dart calls this by itself when the library loads.
///
/// Warnings and errors go to standard error, which Xcode and `flutter run` show, as do
/// Baton's own messages at `info` level. Set `RUST_LOG` to see more.
#[frb(init)]
pub fn init_app() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new("warn,baton_app=info,baton_core=info")
    });
    // A second start in the same process (a hot restart) finds it set up already.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .try_init();
}
