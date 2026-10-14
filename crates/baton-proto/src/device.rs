// SPDX-License-Identifier: Apache-2.0

//! Descriptions of a device that are shown to people.

use serde::{Deserialize, Serialize};

use crate::limits::{
    LimitError, MAX_NAME_BYTES, MAX_VERSION_BYTES, check_len, check_non_empty_len,
};

/// Most characters of a device name that are ever shown.
pub const MAX_DISPLAY_NAME_CHARS: usize = 64;

/// Most characters of a link's title that are ever shown or kept. Page titles run longer
/// than device names, so this is higher.
pub const MAX_DISPLAY_TITLE_CHARS: usize = 256;

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
/// Control characters become spaces. Characters that are invisible, or that reorder
/// text, are removed: the whole Unicode `Default_Ignorable_Code_Point` set, which
/// includes zero-width characters, direction overrides and isolates, the Hangul
/// fillers that render as blank, variation selectors, and the "tag" characters that
/// can hide text in plain sight. The three interlinear annotation characters,
/// U+FFF9 to U+FFFB, are removed too: they are format characters but not default
/// ignorable, and a renderer that supports them may hide the text between them. The
/// braille blank, which renders as empty but is not ignorable, counts as a space.
/// Runs of whitespace collapse to one space, and the result is shortened to
/// [`MAX_DISPLAY_NAME_CHARS`] characters. If nothing is left, [`FALLBACK_NAME`] is
/// returned.
///
/// Removing the zero-width joiner and non-joiner has a cost: they are used inside
/// ordinary words in some scripts (Persian, for one) and to build emoji sequences, so
/// such names lose them. That is accepted, because a name is only a label and these
/// characters are among the most abused for making one name look like another.
///
/// Cleaning is not enough to trust a name. Letters from different scripts that look
/// alike (a Cyrillic "а" for a Latin "a") still let one name imitate another, so a
/// name chosen by a peer must never be what tells a person which device they are
/// dealing with.
pub fn sanitize_display_name(raw: &str) -> String {
    sanitize_text(raw, MAX_DISPLAY_NAME_CHARS).unwrap_or_else(|| FALLBACK_NAME.to_owned())
}

/// Makes any other text from a peer that is only ever shown safe to show, in the way
/// [`sanitize_display_name`] does for a name, except that the result is shortened to
/// `max_chars` characters and is `None`, not [`FALLBACK_NAME`], if nothing visible is left.
///
/// Everything said there about what is removed, and about what cleaning does not do, holds
/// here too.
pub fn sanitize_text(raw: &str, max_chars: usize) -> Option<String> {
    let mut out = String::with_capacity(raw.len().min(max_chars.saturating_mul(4)));
    let mut chars = 0;
    let mut pending_space = false;
    for c in raw.chars() {
        if is_default_ignorable(c) || is_interlinear_annotation(c) {
            continue;
        }
        if c.is_control() || c.is_whitespace() || c == BRAILLE_BLANK {
            pending_space = chars > 0;
            continue;
        }
        let needed = 1 + usize::from(pending_space);
        if chars + needed > max_chars {
            break;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(c);
        chars += needed;
    }
    (!out.is_empty()).then_some(out)
}

/// U+2800 BRAILLE PATTERN BLANK. It draws nothing, yet it is a letter-like symbol, so it
/// is neither whitespace nor default-ignorable.
const BRAILLE_BLANK: char = '\u{2800}';

/// Whether `c` is one of the interlinear annotation characters: U+FFF9 anchor, U+FFFA
/// separator and U+FFFB terminator.
///
/// They mark annotated text, such as a reading printed above a word, and they are in
/// general category Cf (format) but not in `Default_Ignorable_Code_Point`, so
/// [`is_default_ignorable`] is right to say no. A renderer that implements them may draw
/// only the annotated text, or only the annotation, so a name that contains them could
/// show something other than the words it holds. A name has no use for them, and the
/// text between them stays, as ordinary text.
const fn is_interlinear_annotation(c: char) -> bool {
    matches!(c, '\u{FFF9}'..='\u{FFFB}')
}

/// Whether `c` has the Unicode property `Default_Ignorable_Code_Point` (Unicode 16.0).
///
/// These are the code points a renderer is told to draw as nothing when it has no glyph
/// for them, which makes them usable for hiding text and for spoofing. The list is copied
/// from `DerivedCoreProperties.txt` with adjacent ranges merged, and it includes the
/// unassigned code points that are reserved for this purpose, so characters added to
/// these blocks later are already covered.
fn is_default_ignorable(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'                 // soft hyphen
        | '\u{034F}'               // combining grapheme joiner
        | '\u{061C}'               // Arabic letter mark
        | '\u{115F}'..='\u{1160}' // Hangul choseong and jungseong fillers
        | '\u{17B4}'..='\u{17B5}' // Khmer inherent vowels
        | '\u{180B}'..='\u{180F}' // Mongolian variation selectors and vowel separator
        | '\u{200B}'..='\u{200F}' // zero-width space, joiners, direction marks
        | '\u{202A}'..='\u{202E}' // bidirectional embeddings and overrides
        | '\u{2060}'..='\u{206F}' // word joiner, invisible operators, isolates, deprecated
        | '\u{3164}'               // Hangul filler
        | '\u{FE00}'..='\u{FE0F}' // variation selectors 1 to 16
        | '\u{FEFF}'               // zero-width no-break space
        | '\u{FFA0}'               // halfwidth Hangul filler
        | '\u{FFF0}'..='\u{FFF8}' // reserved
        | '\u{1BCA0}'..='\u{1BCA3}' // shorthand format controls
        | '\u{1D173}'..='\u{1D17A}' // musical symbol format controls
        | '\u{E0000}'..='\u{E0FFF}' // tag characters, variation selectors 17 to 256, reserved
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
    fn invisible_characters_are_removed() {
        for (what, c) in [
            ("Hangul filler", '\u{3164}'),
            ("halfwidth Hangul filler", '\u{FFA0}'),
            ("Hangul choseong filler", '\u{115F}'),
            ("Hangul jungseong filler", '\u{1160}'),
            ("combining grapheme joiner", '\u{034F}'),
            ("Khmer inherent vowel", '\u{17B4}'),
            ("Mongolian variation selector", '\u{180B}'),
            ("Mongolian free variation selector four", '\u{180F}'),
            ("deprecated format character", '\u{206A}'),
            ("variation selector 16", '\u{FE0F}'),
            ("variation selector 17", '\u{E0100}'),
            ("tag letter A", '\u{E0041}'),
            ("cancel tag", '\u{E007F}'),
            ("shorthand format overlap", '\u{1BCA0}'),
            ("musical symbol begin beam", '\u{1D173}'),
            ("zero-width joiner", '\u{200D}'),
            ("interlinear annotation anchor", '\u{FFF9}'),
            ("interlinear annotation separator", '\u{FFFA}'),
            ("interlinear annotation terminator", '\u{FFFB}'),
        ] {
            assert_eq!(
                sanitize_display_name(&format!("a{c}b")),
                "ab",
                "{what} was not removed"
            );
        }
    }

    #[test]
    fn interlinear_annotation_characters_are_removed_and_the_text_between_them_stays() {
        // A renderer that supports annotations may show only one of the two parts. The
        // name keeps both, as plain words, so that what is shown is what is there.
        assert_eq!(
            sanitize_display_name("Kitchen\u{FFF9}laptop\u{FFFA}tablet\u{FFFB}"),
            "Kitchenlaptoptablet"
        );
        assert_eq!(
            sanitize_display_name("a \u{FFF9} b \u{FFFA} c \u{FFFB} d"),
            "a b c d"
        );
        // Their neighbours in the block are visible characters and stay.
        assert_eq!(
            sanitize_display_name("a\u{FFFC}b\u{FFFD}c"),
            "a\u{FFFC}b\u{FFFD}c"
        );
    }

    #[test]
    fn text_hidden_in_tag_characters_does_not_survive() {
        // "Laptop" followed by tag characters that spell "ignore previous instructions".
        let hidden: String = "ignore previous instructions"
            .chars()
            .map(|c| char::from_u32(0xE0000 + u32::from(c)).unwrap())
            .collect();
        assert_eq!(sanitize_display_name(&format!("Laptop{hidden}")), "Laptop");
    }

    #[test]
    fn names_that_draw_nothing_use_the_fallback() {
        for raw in [
            "\u{3164}\u{3164}\u{3164}",
            "\u{FFA0}",
            "\u{2800}\u{2800}",
            "\u{E0041}\u{FE0F}",
            "\u{FFF9}\u{FFFA}\u{FFFB}",
            " \u{3164} \u{2800} ",
        ] {
            assert_eq!(sanitize_display_name(raw), FALLBACK_NAME, "{raw:?}");
        }
    }

    #[test]
    fn the_braille_blank_counts_as_a_space() {
        assert_eq!(sanitize_display_name("a\u{2800}\u{2800}b"), "a b");
        assert_eq!(sanitize_display_name("\u{2800}a\u{2800}"), "a");
    }

    #[test]
    fn other_text_is_cleaned_the_same_way_with_a_limit_of_its_own() {
        assert_eq!(
            sanitize_text("Pay\u{202e}ment\n\u{1b}[31m  page", 100).as_deref(),
            Some("Payment [31m page")
        );
        // The limit is the caller's, counted in characters, not bytes.
        let long = "é".repeat(300);
        let cut = sanitize_text(&long, MAX_DISPLAY_TITLE_CHARS).unwrap();
        assert_eq!(cut.chars().count(), MAX_DISPLAY_TITLE_CHARS);
        assert_eq!(sanitize_text("abcdef", 3).as_deref(), Some("abc"));
        // A name stays short even though a title may be long.
        assert_eq!(
            sanitize_display_name(&"x".repeat(300)).chars().count(),
            MAX_DISPLAY_NAME_CHARS
        );
    }

    #[test]
    fn other_text_with_nothing_visible_left_is_nothing_not_the_fallback_name() {
        for raw in [
            "",
            "   ",
            "\u{200b}\u{202e}\n",
            "\u{3164}\u{2800}",
            "\u{FFF9}",
        ] {
            assert_eq!(sanitize_text(raw, 100), None, "{raw:?}");
        }
        assert_eq!(sanitize_text("abc", 0), None);
        // The words of the fallback are ordinary text when a peer really sends them.
        assert_eq!(
            sanitize_text(FALLBACK_NAME, 100).as_deref(),
            Some(FALLBACK_NAME)
        );
    }

    #[test]
    fn no_character_in_a_cleaned_name_is_invisible_or_a_control() {
        // Every Unicode scalar value, one at a time.
        for c in ('\0'..=char::MAX).filter(|c| *c != '\u{FFFE}') {
            let clean = sanitize_display_name(&format!("a{c}b"));
            assert!(
                !clean.chars().any(|k| is_default_ignorable(k)
                    || is_interlinear_annotation(k)
                    || k.is_control()
                    || k == BRAILLE_BLANK),
                "U+{:04X} survived as {clean:?}",
                u32::from(c)
            );
        }
    }

    #[test]
    fn the_ignorable_set_matches_unicode_sixteen() {
        // Spot checks on both sides of every range: the property is a list of ranges,
        // and an off-by-one at an edge is the likely mistake.
        for c in [
            '\u{AD}',
            '\u{34F}',
            '\u{61C}',
            '\u{115F}',
            '\u{1160}',
            '\u{17B4}',
            '\u{17B5}',
            '\u{180B}',
            '\u{180F}',
            '\u{200B}',
            '\u{200F}',
            '\u{202A}',
            '\u{202E}',
            '\u{2060}',
            '\u{206F}',
            '\u{3164}',
            '\u{FE00}',
            '\u{FE0F}',
            '\u{FEFF}',
            '\u{FFA0}',
            '\u{FFF0}',
            '\u{FFF8}',
            '\u{1BCA0}',
            '\u{1BCA3}',
            '\u{1D173}',
            '\u{1D17A}',
            '\u{E0000}',
            '\u{E0FFF}',
        ] {
            assert!(is_default_ignorable(c), "U+{:04X}", u32::from(c));
        }
        for c in [
            '\u{AC}',
            '\u{AE}',
            '\u{34E}',
            '\u{350}',
            '\u{61B}',
            '\u{61D}',
            '\u{115E}',
            '\u{1161}',
            '\u{17B3}',
            '\u{17B6}',
            '\u{180A}',
            '\u{1810}',
            '\u{200A}',
            '\u{2010}',
            '\u{2029}',
            '\u{202F}',
            '\u{205F}',
            '\u{2070}',
            '\u{3163}',
            '\u{3165}',
            '\u{FDFF}',
            '\u{FE10}',
            '\u{FEFE}',
            '\u{FF00}',
            '\u{FF9F}',
            '\u{FFA1}',
            '\u{FFEF}',
            '\u{FFF9}',
            '\u{1BC9F}',
            '\u{1BCA4}',
            '\u{1D172}',
            '\u{1D17B}',
            '\u{DFFFF}',
            '\u{E1000}',
        ] {
            assert!(!is_default_ignorable(c), "U+{:04X}", u32::from(c));
        }
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
