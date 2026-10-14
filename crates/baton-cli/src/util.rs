// SPDX-License-Identifier: Apache-2.0

//! Small helpers shared by the commands.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch, by this device's clock.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Whether a typed answer is a yes: `y` or `yes`, in any case. Anything else, and no
/// answer at all, is a no.
pub fn is_yes(answer: Option<&str>) -> bool {
    answer.is_some_and(|answer| {
        let answer = answer.trim();
        answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_yes_is_yes() {
        for yes in ["y", "Y", "yes", "YES", " Yes \n"] {
            assert!(is_yes(Some(yes)), "{yes:?}");
        }
        for no in ["", "n", "no", "yess", "yep", "ok", "1", "y e s"] {
            assert!(!is_yes(Some(no)), "{no:?}");
        }
        assert!(!is_yes(None));
    }

    #[test]
    fn the_clock_is_after_2025() {
        assert!(unix_now() > 1_735_689_600);
    }
}
