// SPDX-License-Identifier: Apache-2.0

//! What this device does with things a paired device hands it, and what it sends.
//!
//! Policies are kept per paired device and apply on the *receiving* side. The
//! defaults are cautious: nothing is opened or overwritten until the person
//! chooses. See `docs/security.md` for the reasoning.

use serde::{Deserialize, Serialize};

/// What to do with a link a paired device sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkPolicy {
    /// Keep it in the inbox until the person opens it.
    #[default]
    Inbox,
    /// Open it in the default browser straight away. Only `http` and `https` links are ever opened.
    Open,
    /// Discard links from this device.
    Ignore,
}

/// What to do with clipboard text a paired device sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClipboardPolicy {
    /// Keep it in the inbox until the person copies it.
    #[default]
    Inbox,
    /// Replace the clipboard with it straight away.
    Apply,
    /// Discard clipboard text from this device.
    Ignore,
}

/// Whether a paired device may ask this one to start sharing its screen with it.
///
/// The question is asked once, here, by the person at this device, instead of each time the paired
/// device wants to look (`docs/adr/0010-sessions-that-the-viewer-starts.md`). Only the person at
/// this device can change it: nothing a paired device sends can.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StartPolicy {
    /// No. A paired device that asks is told that the settings do not let it.
    #[default]
    Off,
    /// Yes, to look at the screen without a person here being asked each time. The paired device
    /// cannot control it.
    Look,
    /// Yes, to look at the screen and to control it, without a person here being asked each time.
    Control,
}

impl StartPolicy {
    /// Whether the paired device may start a session at all.
    #[must_use]
    pub const fn allows_start(self) -> bool {
        !matches!(self, Self::Off)
    }

    /// Whether the paired device may control what it sees.
    #[must_use]
    pub const fn allows_control(self) -> bool {
        matches!(self, Self::Control)
    }
}

/// The settings for one paired device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DevicePolicy {
    /// What to do with links this device sends.
    #[serde(default)]
    pub links: LinkPolicy,
    /// What to do with clipboard text this device sends.
    #[serde(default)]
    pub clipboard: ClipboardPolicy,
    /// Send this device every change to the clipboard on this machine.
    ///
    /// Off by default, because the clipboard often holds secrets.
    #[serde(default)]
    pub sync_clipboard_to: bool,
    /// Whether this device may ask that one to start sharing its screen with it.
    ///
    /// Off by default, because it lets the paired device look at everything on the screen, and
    /// perhaps control it, without anyone here being asked.
    #[serde(default)]
    pub start: StartPolicy,
    /// Whether this device may hear the sound of this machine's screen, and so make this machine
    /// quiet while it listens. `None` when the person here has not said, and then it may in a
    /// session that lets it control the screen and may not in one that lets it only look
    /// ([`may_hear`](Self::may_hear)).
    ///
    /// Not kept at all while nobody has said, so that a file stays as an older version wrote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hear: Option<bool>,
}

impl DevicePolicy {
    /// Whether this device may hear the sound of the screen of this machine in a session, which
    /// lets it control the screen if `controls`: what the person here said of it, and when they
    /// have not, whether the session lets it control. A device that can only look is not to
    /// silence this machine by listening unless they say so.
    ///
    /// What the device may ask for in general ([`start`](Self::start)) does not enter into it. A
    /// session is as the person here made it or allowed it to be made, whoever started it: a
    /// screen that they share with control to a device that may not ask for it is heard, and one
    /// that they share for looking is not heard by a device that may ask for control.
    #[must_use]
    pub const fn may_hear(&self, controls: bool) -> bool {
        match self.hear {
            Some(hear) => hear,
            None => controls,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_cautious() {
        let policy = DevicePolicy::default();
        assert_eq!(policy.links, LinkPolicy::Inbox);
        assert_eq!(policy.clipboard, ClipboardPolicy::Inbox);
        assert!(!policy.sync_clipboard_to);
        assert_eq!(policy.start, StartPolicy::Off);
        assert_eq!(policy.hear, None);
        // Nothing is said of a device just paired: it does not hear where it can only look.
        assert!(!policy.may_hear(false));
    }

    #[test]
    fn a_device_hears_when_the_person_said_so_and_otherwise_when_the_session_lets_it_control() {
        for (hear, controls, heard) in [
            // Not said: it follows the session.
            (None, false, false),
            (None, true, true),
            // Said: that is all that counts, either way.
            (Some(true), false, true),
            (Some(true), true, true),
            (Some(false), false, false),
            (Some(false), true, false),
        ] {
            // What the device may ask for in general makes no difference to any of them.
            for start in [StartPolicy::Off, StartPolicy::Look, StartPolicy::Control] {
                let policy = DevicePolicy {
                    start,
                    hear,
                    ..DevicePolicy::default()
                };
                assert_eq!(
                    policy.may_hear(controls),
                    heard,
                    "{start:?}, said {hear:?}, controls {controls}"
                );
            }
        }
    }

    #[test]
    fn the_hear_setting_is_kept_only_once_it_is_said_and_an_old_file_without_it_means_not_said() {
        // Nobody said: nothing is written, so an older version reads the file as it wrote it.
        let json = serde_json::to_string(&DevicePolicy::default()).unwrap();
        assert!(!json.contains("hear"), "{json}");
        for hear in [true, false] {
            let policy = DevicePolicy {
                hear: Some(hear),
                ..DevicePolicy::default()
            };
            let json = serde_json::to_string(&policy).unwrap();
            assert!(json.contains(&format!("\"hear\":{hear}")), "{json}");
            assert_eq!(serde_json::from_str::<DevicePolicy>(&json).unwrap(), policy);
        }
        let policy: DevicePolicy = serde_json::from_str(r#"{"start":"control"}"#).unwrap();
        assert_eq!(policy.hear, None);
        assert!(policy.may_hear(true));
        assert!(!policy.may_hear(false));
        // A value that is not a yes or a no is an error, not a guess at what was meant.
        assert!(serde_json::from_str::<DevicePolicy>(r#"{"hear":"yes"}"#).is_err());
    }

    #[test]
    fn starting_a_session_is_off_until_the_person_here_turns_it_on() {
        assert!(!StartPolicy::Off.allows_start());
        assert!(!StartPolicy::Off.allows_control());
        assert!(StartPolicy::Look.allows_start());
        assert!(!StartPolicy::Look.allows_control());
        assert!(StartPolicy::Control.allows_start());
        assert!(StartPolicy::Control.allows_control());
    }

    #[test]
    fn the_start_setting_is_kept_by_name_and_an_old_file_without_it_means_off() {
        for (text, start) in [
            ("off", StartPolicy::Off),
            ("look", StartPolicy::Look),
            ("control", StartPolicy::Control),
        ] {
            let json = format!(r#"{{"start":"{text}"}}"#);
            let policy: DevicePolicy = serde_json::from_str(&json).unwrap();
            assert_eq!(policy.start, start);
            assert_eq!(
                serde_json::to_string(&policy.start).unwrap(),
                format!("\"{text}\"")
            );
        }
        // A file written before the setting existed has none, and nothing is allowed.
        let policy: DevicePolicy = serde_json::from_str(r#"{"links":"open"}"#).unwrap();
        assert_eq!(policy.start, StartPolicy::Off);
        // A value that is not one of the three is an error, not a guess at what was meant.
        assert!(serde_json::from_str::<DevicePolicy>(r#"{"start":"always"}"#).is_err());
        assert!(serde_json::from_str::<DevicePolicy>(r#"{"start":true}"#).is_err());
    }

    #[test]
    fn missing_fields_fall_back_to_the_defaults() {
        let policy: DevicePolicy = serde_json::from_str(r#"{"links":"open"}"#).unwrap();
        assert_eq!(policy.links, LinkPolicy::Open);
        assert_eq!(policy.clipboard, ClipboardPolicy::Inbox);
        let policy: DevicePolicy = serde_json::from_str("{}").unwrap();
        assert_eq!(policy, DevicePolicy::default());
    }

    #[test]
    fn unknown_values_are_errors_not_guesses() {
        assert!(serde_json::from_str::<DevicePolicy>(r#"{"links":"always"}"#).is_err());
    }
}
