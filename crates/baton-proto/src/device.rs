// SPDX-License-Identifier: Apache-2.0

//! Descriptions of a device that are shown to people.

use serde::{Deserialize, Serialize};

use crate::limits::{
    LimitError, MAX_NAME_BYTES, MAX_VERSION_BYTES, check_len, check_non_empty_len,
};

/// Most characters of a device name that are ever shown.
pub const MAX_DISPLAY_NAME_CHARS: usize = 64;

/// Shown when a peer's name is empty once cleaned up.
pub const FALLBACK_NAME: &str = "unnamed device";

/// The operating system a device runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Platform {
    /// macOS.
    MacOs,
    /// Windows.
    Windows,
    /// Linux.
    Linux,
    /// iOS.
    Ios,
    /// iPadOS.
    IpadOs,
    /// Android.
    Android,
    /// Anything this version does not know about.
    #[serde(other)]
    Other,
}

impl Platform {
    /// The platform this build is running on.
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "ios") {
            Self::Ios
        } else if cfg!(target_os = "android") {
            Self::Android
        } else {
            Self::Other
        }
    }
}

/// What a device says about itself.
///
/// All of it is chosen by the peer, so it is only ever used for display, after
/// [`sanitize_display_name`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// A name the person recognises, such as "Kitchen laptop".
    pub name: String,
    /// The operating system.
    pub platform: Platform,
    /// The version of the app, for diagnostics.
    pub app_version: String,
}

impl DeviceInfo {
    /// Checks the field limits.
    pub fn validate(&self) -> Result<(), LimitError> {
        check_non_empty_len("device name", &self.name, MAX_NAME_BYTES)?;
        check_len("app version", &self.app_version, MAX_VERSION_BYTES)
    }
}

/// Makes a name that came from a peer safe to show in a prompt or write to a log.
///
/// Control characters become spaces. Characters that reorder or hide text, which
/// can make one name look like another, are removed. Runs of whitespace collapse
/// to one space, and the result is shortened to [`MAX_DISPLAY_NAME_CHARS`]
/// characters. If nothing is left, [`FALLBACK_NAME`] is returned.
pub fn sanitize_display_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(MAX_NAME_BYTES));
    let mut chars = 0;
    let mut pending_space = false;
    for c in raw.chars() {
        if is_spoofing_format_char(c) {
            continue;
        }
        if c.is_control() || c.is_whitespace() {
            pending_space = chars > 0;
            continue;
        }
        let needed = 1 + usize::from(pending_space);
        if chars + needed > MAX_DISPLAY_NAME_CHARS {
            break;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(c);
        chars += needed;
    }
    if out.is_empty() {
        FALLBACK_NAME.to_owned()
    } else {
        out
    }
}

/// Invisible and direction-changing characters that are removed from names.
fn is_spoofing_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'                // soft hyphen
        | '\u{061C}'              // Arabic letter mark
        | '\u{180E}'              // Mongolian vowel separator
        | '\u{200B}'..='\u{200F}' // zero-width space and joiners, LRM, RLM
        | '\u{202A}'..='\u{202E}' // bidirectional embeddings and overrides
        | '\u{2060}'..='\u{2064}' // word joiner and invisible operators
        | '\u{2066}'..='\u{2069}' // bidirectional isolates
        | '\u{FEFF}'              // zero-width no-break space
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_pass_through() {
        assert_eq!(
            sanitize_display_name("Jay's MacBook Pro"),
            "Jay's MacBook Pro"
        );
        assert_eq!(sanitize_display_name("書斎のiPad"), "書斎のiPad");
    }

    #[test]
    fn whitespace_collapses_and_trims() {
        assert_eq!(sanitize_display_name("  a \t\n b   c  "), "a b c");
    }

    #[test]
    fn control_characters_cannot_forge_extra_lines() {
        assert_eq!(
            sanitize_display_name("Laptop\r\nAccept? [y/N]"),
            "Laptop Accept? [y/N]"
        );
        assert_eq!(sanitize_display_name("a\u{1b}[31mred"), "a [31mred");
    }

    #[test]
    fn direction_overrides_are_removed() {
        assert_eq!(sanitize_display_name("abc\u{202E}fed"), "abcfed");
        assert_eq!(sanitize_display_name("a\u{2066}b\u{2069}c"), "abc");
        assert_eq!(sanitize_display_name("zero\u{200B}width"), "zerowidth");
    }

    #[test]
    fn empty_results_use_the_fallback() {
        for raw in ["", "   ", "\u{200B}\u{202E}", "\n\t"] {
            assert_eq!(sanitize_display_name(raw), FALLBACK_NAME);
        }
    }

    #[test]
    fn long_names_are_cut_by_characters_not_bytes() {
        let raw = "é".repeat(200);
        let clean = sanitize_display_name(&raw);
        assert_eq!(clean.chars().count(), MAX_DISPLAY_NAME_CHARS);
        let raw = format!("{} {}", "a".repeat(63), "b");
        assert_eq!(sanitize_display_name(&raw).chars().count(), 63);
    }

    #[test]
    fn unknown_platforms_decode_as_other() {
        let cbor = {
            let mut out = Vec::new();
            ciborium::into_writer(&"PlanNine", &mut out).unwrap();
            out
        };
        let platform: Platform = ciborium::from_reader(cbor.as_slice()).unwrap();
        assert_eq!(platform, Platform::Other);
    }

    #[test]
    fn device_info_limits() {
        let ok = DeviceInfo {
            name: "x".into(),
            platform: Platform::Linux,
            app_version: "0.1.0".into(),
        };
        assert!(ok.validate().is_ok());
        assert!(
            DeviceInfo {
                name: String::new(),
                ..ok.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            DeviceInfo {
                name: "n".repeat(MAX_NAME_BYTES + 1),
                ..ok.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            DeviceInfo {
                app_version: "v".repeat(MAX_VERSION_BYTES + 1),
                ..ok
            }
            .validate()
            .is_err()
        );
    }
}
