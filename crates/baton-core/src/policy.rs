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
