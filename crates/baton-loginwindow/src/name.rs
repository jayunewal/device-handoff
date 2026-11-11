// SPDX-License-Identifier: Apache-2.0

//! What a phone calls the program: the name of the Mac, and where it runs.

use baton_app::settings;

use crate::{Error, ErrorKind, Result, State};

/// What follows the name of the Mac, so that a phone that lists the Mac twice, once for the app
/// and once for this program, shows which is which.
const SUFFIX: &str = " (login window)";

/// The most of the Mac's own name that is kept. The name that a device gives itself is cut at 64
/// characters when it is shown, and the end that says where it runs must stay.
const MAX_MAC_NAME_CHARS: usize = 40;

/// The name that devices see for the program of a Mac called `mac_name`.
///
/// A name that the Mac has by default, such as `Jays-MBP.local`, loses the `.local` that the
/// network adds, and an empty one is called `Mac`.
pub(crate) fn login_window_name(mac_name: &str) -> String {
    let mac_name = mac_name.trim();
    let mac_name = mac_name.strip_suffix(".local").unwrap_or(mac_name).trim();
    let mac_name: String = mac_name.chars().take(MAX_MAC_NAME_CHARS).collect();
    let mac_name = mac_name.trim();
    if mac_name.is_empty() {
        format!("Mac{SUFFIX}")
    } else {
        format!("{mac_name}{SUFFIX}")
    }
}

/// Saves the name that devices see for the program.
///
/// With `mac_name`, that is the name of this Mac, which the phone shows with ` (login window)`
/// after it. Without it, a name that is saved already stays, and one that is not is made from the
/// host name of the computer. The install script gives the name that the person chose for the Mac
/// in System Settings.
///
/// # Errors
///
/// [`ErrorKind::Other`] if the name has nothing visible in it or the settings cannot be saved.
pub fn name_this_mac(state: &State, mac_name: Option<&str>) -> Result<()> {
    let name = if let Some(given) = mac_name {
        login_window_name(given)
    } else {
        let saved = settings::device_name(state.path()).map_err(Error::from)?;
        if saved.is_some() {
            return Ok(());
        }
        login_window_name(&gethostname::gethostname().to_string_lossy())
    };
    settings::set_device_name(state.path(), &name).map_err(|error| {
        Error::new(
            ErrorKind::Other,
            format!("The name {name:?} cannot be used: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_says_where_it_runs() {
        assert_eq!(
            login_window_name("Jay's MacBook Pro"),
            "Jay's MacBook Pro (login window)"
        );
        assert_eq!(login_window_name("  Studio  "), "Studio (login window)");
    }

    #[test]
    fn the_name_that_the_network_adds_is_left_off() {
        assert_eq!(
            login_window_name("Jays-MBP.local"),
            "Jays-MBP (login window)"
        );
    }

    #[test]
    fn a_name_with_nothing_in_it_is_called_mac() {
        for empty in ["", "   ", ".local"] {
            assert_eq!(login_window_name(empty), "Mac (login window)", "{empty:?}");
        }
    }

    #[test]
    fn a_long_name_is_cut_where_it_starts_and_the_end_stays() {
        let name = login_window_name(&"x".repeat(200));
        assert!(name.ends_with(SUFFIX), "{name}");
        assert!(name.chars().count() <= 64, "{name}");
        assert_eq!(
            name.chars().count(),
            MAX_MAC_NAME_CHARS + SUFFIX.chars().count()
        );
    }

    #[test]
    fn the_name_is_kept_in_the_settings_and_a_given_one_replaces_it() {
        let parent = tempfile::tempdir().unwrap();
        let state = State::open(Some(parent.path().join("state"))).unwrap();

        name_this_mac(&state, None).unwrap();
        let made = settings::device_name(state.path()).unwrap().unwrap();
        assert!(made.ends_with(SUFFIX), "{made}");

        // A name that is saved stays when none is given.
        name_this_mac(&state, Some("Kitchen")).unwrap();
        name_this_mac(&state, None).unwrap();
        assert_eq!(
            settings::device_name(state.path()).unwrap().as_deref(),
            Some("Kitchen (login window)")
        );
    }
}
