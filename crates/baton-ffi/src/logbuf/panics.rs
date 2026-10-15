// SPDX-License-Identifier: Apache-2.0

//! Panics, in the log.
//!
//! A panic prints its message to standard error and nowhere else. On a phone, or in a Mac app
//! opened from the Finder, nobody sees that, and a panic is what a bug report most needs to
//! have. [`log_panics`] makes the message, and the place it came from, a line of the log as
//! well, where it is made safe like every other line.

use std::panic::{self, PanicHookInfo};

/// Makes every panic from now on a line of the log. The hook that was there before still gets
/// to see it, so standard error shows what it showed.
pub(crate) fn log_panics() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        // A panic in the code that makes lines safe is not sent through it again (see
        // `is_cleaning`).
        if !super::is_cleaning() {
            tracing::error!("{}", describe(info));
        }
        previous(info);
    }));
}

/// What a panic says in one line: where it happened and the message it came with.
fn describe(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("no message");
    match info.location() {
        Some(place) => format!("panicked at {place}: {message}"),
        None => format!("panicked: {message}"),
    }
}
