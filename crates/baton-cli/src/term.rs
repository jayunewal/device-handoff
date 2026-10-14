// SPDX-License-Identifier: Apache-2.0

//! Turning what other devices sent into text that is safe and pleasant to print.
//!
//! A received link, title or piece of text is chosen by another device. Printed as it
//! is, it could move the cursor, clear the screen, change the terminal's title or draw
//! over what the person is reading: escape sequences start with a control character,
//! and some terminals treat the C1 range (U+0080 to U+009F) as controls too. Text that
//! reorders itself (direction overrides) can make a link read differently from what it
//! is. Everything here replaces those characters with a visible escape such as
//! `\u{1b}`, so the person sees that they were there.

use std::fmt::Write as _;

use baton_proto::Platform;

/// Whether `c` must not reach a terminal as it is.
fn needs_escape(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061C}'                 // Arabic letter mark
            | '\u{200E}' | '\u{200F}' // direction marks
            | '\u{2028}' | '\u{2029}' // line and paragraph separators
            | '\u{202A}'..='\u{202E}' // embeddings and overrides
            | '\u{2066}'..='\u{2069}' // isolates
        )
}

fn push_escaped(out: &mut String, c: char) {
    // Writing to a String cannot fail.
    let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
}

/// `text` on one line: line breaks and tabs become `\n`, `\r` and `\t`, and every other
/// character that could act on a terminal becomes `\u{..}`.
pub fn line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if needs_escape(c) => push_escaped(&mut out, c),
            c => out.push(c),
        }
    }
    out
}

/// `text` over several lines: line breaks and tabs stay, and every other character that
/// could act on a terminal becomes `\u{..}`. A lone `\r` is escaped too, since it would
/// send the cursor back to the start of the line and let later text overwrite earlier.
pub fn block(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\n' | '\t' => out.push(c),
            c if needs_escape(c) => push_escaped(&mut out, c),
            c => out.push(c),
        }
    }
    out
}

/// The start of `text` for a list: the first line, escaped, and cut to `max_chars`
/// characters. A cut, or more lines after the first, is shown by `…`.
pub fn preview(text: &str, max_chars: usize) -> String {
    let text = text.trim_start_matches(['\n', '\r']);
    let first = text.split(['\n', '\r']).next().unwrap_or_default();
    let more_lines = first.len() < text.trim_end().len();
    let escaped = line(first);

    let keep = max_chars.saturating_sub(1);
    let mut chars = escaped.chars();
    let shown: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        // Cut: leave room for the ellipsis. Escapes such as `\u{1b}` may be cut in the
        // middle, which is fine for a preview.
        let mut cut: String = shown.chars().take(keep).collect();
        cut.push('…');
        cut
    } else if more_lines {
        format!("{shown}…")
    } else {
        shown
    }
}

/// How long ago `then` was, as of `now` (both in seconds since the Unix epoch).
pub fn ago(now: u64, then: u64) -> String {
    let secs = now.saturating_sub(then);
    match secs {
        0..=9 => "just now".to_owned(),
        10..=59 => format!("{secs} s ago"),
        60..=3_599 => format!("{} min ago", secs / 60),
        3_600..=86_399 => format!("{} h ago", secs / 3_600),
        86_400..=604_799 => format!("{} d ago", secs / 86_400),
        _ => utc_date(then),
    }
}

/// `YYYY-MM-DD` in UTC for a Unix time.
fn utc_date(unix: u64) -> String {
    // The civil-from-days algorithm (Howard Hinnant, public domain) on days since 1970-01-01.
    let days = i64::try_from(unix / 86_400).unwrap_or(i64::MAX / 4);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// A size for people: `42 bytes`, `1.5 KiB`.
pub fn size(bytes: usize) -> String {
    if bytes == 1 {
        "1 byte".to_owned()
    } else if bytes < 1024 {
        format!("{bytes} bytes")
    } else {
        // Sizes here are at most a few megabytes, far inside f64's exact range.
        #[allow(clippy::cast_precision_loss)]
        let kib = bytes as f64 / 1024.0;
        format!("{kib:.1} KiB")
    }
}

/// The name of an operating system, as a person would say it.
pub const fn platform(platform: Platform) -> &'static str {
    match platform {
        Platform::MacOs => "macOS",
        Platform::Windows => "Windows",
        Platform::Linux => "Linux",
        Platform::Ios => "iOS",
        Platform::IpadOs => "iPadOS",
        Platform::Android => "Android",
        Platform::Other => "unknown",
    }
}

/// Rows of text laid out in columns, each as wide as its widest cell, with two spaces
/// between columns and no trailing spaces on a line.
pub fn table(rows: &[Vec<String>]) -> String {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut widths = vec![0; columns];
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let mut out = String::new();
    for row in rows {
        let mut line = String::new();
        for (cell, width) in row.iter().zip(&widths) {
            // Writing to a String cannot fail.
            let _ = write!(line, "{cell:<width$}  ");
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_sequences_cannot_reach_a_terminal() {
        let attack = "before\u{1b}[2J\u{1b}]0;title\u{7}after";
        let shown = line(attack);
        assert!(!shown.contains('\u{1b}'));
        assert!(!shown.contains('\u{7}'));
        assert_eq!(shown, "before\\u{1b}[2J\\u{1b}]0;title\\u{7}after");
        let shown = block(attack);
        assert!(!shown.contains('\u{1b}'));
    }

    #[test]
    fn the_c1_controls_and_direction_overrides_are_escaped() {
        // U+009B is a single-character CSI on some terminals.
        assert_eq!(line("a\u{9b}31m"), "a\\u{9b}31m");
        assert_eq!(line("a\u{202e}b"), "a\\u{202e}b");
        assert_eq!(line("a\u{2067}b\u{2069}"), "a\\u{2067}b\\u{2069}");
        assert_eq!(block("x\u{2028}y"), "x\\u{2028}y");
    }

    #[test]
    fn ordinary_text_passes_through() {
        let text = "Grüße, 世界! 🎉 https://example.com/a?b=c#d";
        assert_eq!(line(text), text);
        assert_eq!(block(text), text);
    }

    #[test]
    fn a_line_shows_its_breaks_and_a_block_keeps_them() {
        assert_eq!(line("a\nb\r\nc\td"), "a\\nb\\r\\nc\\td");
        assert_eq!(block("a\nb\r\nc\td"), "a\nb\nc\td");
    }

    #[test]
    fn a_lone_carriage_return_cannot_overwrite_earlier_text_in_a_block() {
        assert_eq!(block("real\rfake"), "real\\u{d}fake");
    }

    #[test]
    fn a_preview_is_one_short_line() {
        assert_eq!(preview("hello", 20), "hello");
        assert_eq!(preview("hello", 5), "hello");
        assert_eq!(preview("hello world", 6), "hello…");
        assert_eq!(preview("first\nsecond", 20), "first…");
        assert_eq!(preview("\n\nfirst", 20), "first");
        assert_eq!(preview("", 20), "");
        assert_eq!(preview("tail\n", 20), "tail");
    }

    #[test]
    fn a_preview_counts_characters_not_bytes() {
        assert_eq!(preview("日本語のテキスト", 4), "日本語…");
        assert_eq!(preview("日本語", 3), "日本語");
    }

    #[test]
    fn a_preview_escapes() {
        assert_eq!(preview("a\u{1b}[31mred", 40), "a\\u{1b}[31mred");
    }

    #[test]
    fn ages_are_short_and_rounded_down() {
        let now = 1_800_000_000;
        assert_eq!(ago(now, now), "just now");
        assert_eq!(ago(now, now + 100), "just now");
        assert_eq!(ago(now, now - 30), "30 s ago");
        assert_eq!(ago(now, now - 60), "1 min ago");
        assert_eq!(ago(now, now - 3_599), "59 min ago");
        assert_eq!(ago(now, now - 3_600), "1 h ago");
        assert_eq!(ago(now, now - 86_399), "23 h ago");
        assert_eq!(ago(now, now - 86_400), "1 d ago");
        assert_eq!(ago(now, now - 6 * 86_400), "6 d ago");
    }

    #[test]
    fn old_items_show_a_date() {
        // 2027-01-15 08:00:00 UTC.
        let then = 1_799_999_999 - 86_400 * 8;
        let now = 1_800_000_000;
        assert_eq!(ago(now, then), utc_date(then));
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(951_782_400), "2000-02-29");
        assert_eq!(utc_date(1_800_000_000), "2027-01-15");
        assert_eq!(utc_date(4_102_444_800), "2100-01-01");
    }

    #[test]
    fn sizes_are_in_bytes_until_a_kilobyte() {
        assert_eq!(size(0), "0 bytes");
        assert_eq!(size(1), "1 byte");
        assert_eq!(size(1023), "1023 bytes");
        assert_eq!(size(1024), "1.0 KiB");
        assert_eq!(size(1536), "1.5 KiB");
    }

    #[test]
    fn tables_align_columns_and_leave_no_trailing_spaces() {
        let row = |cells: [&str; 3]| cells.map(str::to_owned).to_vec();
        let rows = vec![
            row(["NAME", "SYSTEM", "SEEN"]),
            row(["Laptop", "macOS", "now"]),
            row(["PC", "Windows", ""]),
        ];
        assert_eq!(
            table(&rows),
            "NAME    SYSTEM   SEEN\nLaptop  macOS    now\nPC      Windows\n"
        );
        assert_eq!(table(&[]), "");
    }
}
