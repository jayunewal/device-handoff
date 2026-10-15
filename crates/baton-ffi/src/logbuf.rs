// SPDX-License-Identifier: Apache-2.0

//! The last lines Baton logged, kept in memory so that the app can show them.
//!
//! On a phone, or in a Mac app that was opened from the Finder, nobody sees standard error, so
//! a person who has a problem can only say what the screen said. The app therefore keeps the
//! last few hundred log lines and shows them on its Diagnostics screen, where they can be copied
//! into an issue. A line is made safe for that on the way in, because these lines get pasted
//! into a public place: it is cut to a length, characters that would act on a screen instead of
//! showing are replaced, anything that looks like a pairing code is removed together with the
//! rest of the event (a code is a bearer secret while its offer lasts), and so are the details that say who a device belongs to or
//! where it is: whole keys, the account name in a path and public IP addresses (see [`hide`]).

mod hide;

use std::{
    borrow::Cow,
    collections::VecDeque,
    io,
    sync::{Mutex, PoisonError},
};

use tracing_subscriber::fmt::MakeWriter;

/// How many lines are kept. The oldest go first.
const CAPACITY: usize = 400;

/// The most characters of a line that are kept. What is cut off is replaced by `…`.
const LINE_LIMIT: usize = 600;

/// What a pairing code that reached a log line is replaced by, and with it the rest of the
/// event.
const CODE_REMOVED: &str = "BATON1:[removed]";

static LOG: LogBuffer = LogBuffer::new(CAPACITY);

/// The lines logged so far, oldest first. At most [`CAPACITY`] of them.
pub(crate) fn recent() -> Vec<String> {
    LOG.lines()
}

/// Where the logging setup sends a copy of every line it prints.
pub(crate) fn capture() -> Capture {
    Capture(&LOG)
}

/// A bounded list of log lines.
#[derive(Debug)]
pub(crate) struct LogBuffer {
    lines: Mutex<VecDeque<String>>,
    capacity: usize,
}

impl LogBuffer {
    pub(crate) const fn new(capacity: usize) -> Self {
        Self {
            lines: Mutex::new(VecDeque::new()),
            capacity,
        }
    }

    /// Keeps the lines of `text`, made safe, and drops the oldest ones above the capacity.
    pub(crate) fn push(&self, text: &str) {
        // The text is one event, and the parser of a pairing code ignores white space, so a
        // code can run over several lines. What follows one in the event goes with it.
        let text = remove_codes(text);
        // A panic while the list was held leaves it as it was, so a poisoned lock is still
        // good: a log that stopped working is the last thing to want while looking for a bug.
        let mut lines = self.lines.lock().unwrap_or_else(PoisonError::into_inner);
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            while lines.len() >= self.capacity.max(1) {
                lines.pop_front();
            }
            lines.push_back(clean(line));
        }
    }

    /// The lines kept, oldest first.
    pub(crate) fn lines(&self) -> Vec<String> {
        let lines = self.lines.lock().unwrap_or_else(PoisonError::into_inner);
        lines.iter().cloned().collect()
    }
}

/// What the formatting layer of `tracing` writes into. A line is kept when the writer is
/// dropped, which is when `tracing` has written all of an event.
#[derive(Debug)]
pub(crate) struct LineWriter<'a> {
    buffer: &'a LogBuffer,
    bytes: Vec<u8>,
}

impl io::Write for LineWriter<'_> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for LineWriter<'_> {
    fn drop(&mut self) {
        if !self.bytes.is_empty() {
            self.buffer.push(&String::from_utf8_lossy(&self.bytes));
        }
    }
}

/// Hands out a [`LineWriter`] for every event, for `tracing_subscriber::fmt`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Capture(&'static LogBuffer);

impl<'a> MakeWriter<'a> for Capture {
    type Writer = LineWriter<'a>;

    fn make_writer(&'a self) -> LineWriter<'a> {
        LineWriter {
            buffer: self.0,
            bytes: Vec::new(),
        }
    }
}

/// `line` with keys, account names and public addresses taken out, characters that would act
/// on a screen replaced, and a length that is not too long.
fn clean(line: &str) -> String {
    let line = hide::shorten_keys(line);
    let line = hide::hide_account_names(&line);
    let line = hide::hide_public_addresses(&line);
    let mut cleaned = String::with_capacity(line.len().min(LINE_LIMIT));
    for (count, c) in line.chars().enumerate() {
        if count == LINE_LIMIT {
            cleaned.push('…');
            break;
        }
        cleaned.push(if hidden(c) {
            char::REPLACEMENT_CHARACTER
        } else {
            c
        });
    }
    cleaned
}

/// Whether `c` cannot be seen or would turn text round: a control character other than the tab,
/// the direction marks, embeddings, overrides and isolates, the zero-width space, the word
/// joiner and the invisible operators, the deprecated format characters, the byte order mark and
/// the annotation marks. The joiners that emoji and several scripts need (U+200C and U+200D) and
/// the variation selectors are not on the list. The app shows received text the same way
/// (`visibleText` in `lib/src/format.dart`) and the two lists are the same.
fn hidden(c: char) -> bool {
    (c.is_control() && c != '\t')
        || matches!(
            c,
            '\u{61C}'
                | '\u{200B}'
                | '\u{200E}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
        )
}

/// `text` with the first pairing code (`BATON1:`) and everything after it replaced by
/// [`CODE_REMOVED`]. The parser of a ticket ignores case and white space, so a code can be
/// written in groups or over several lines, and only the end of the event is a place where it
/// surely ends.
fn remove_codes(text: &str) -> Cow<'_, str> {
    const PREFIX: &str = "baton1:";
    // Lowercasing the ASCII letters keeps every byte where it was, so a position found in the
    // lowercase copy is a position in the text, and it is a character boundary there because
    // the match is ASCII.
    match text.to_ascii_lowercase().find(PREFIX) {
        None => Cow::Borrowed(text),
        Some(start) => Cow::Owned(format!("{}{CODE_REMOVED}", &text[..start])),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    /// A buffer that lives for the whole test run, as the real one does.
    fn buffer(capacity: usize) -> &'static LogBuffer {
        Box::leak(Box::new(LogBuffer::new(capacity)))
    }

    #[test]
    fn keeps_the_newest_lines_in_order() {
        let log = buffer(3);

        for n in 1..=5 {
            log.push(&format!("line {n}\n"));
        }

        assert_eq!(log.lines(), ["line 3", "line 4", "line 5"]);
    }

    #[test]
    fn a_text_of_several_lines_is_several_lines_and_blank_ones_are_dropped() {
        let log = buffer(10);

        log.push("first\r\n\r\n   \nsecond\n");

        assert_eq!(log.lines(), ["first", "second"]);
    }

    #[test]
    fn a_log_that_may_hold_nothing_still_holds_the_last_line() {
        let log = buffer(0);

        log.push("a\nb\n");

        assert_eq!(log.lines(), ["b"]);
    }

    #[test]
    fn a_line_is_cut_to_its_limit_and_says_so() {
        let log = buffer(10);

        // Multi-byte characters, so that a cut in the middle of one would panic.
        log.push(&"é".repeat(LINE_LIMIT + 50));

        let lines = log.lines();
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert_eq!(line.chars().count(), LINE_LIMIT + 1);
        assert!(line.ends_with('…'));
        assert!(line.starts_with("éé"));
    }

    #[test]
    fn a_line_exactly_at_the_limit_is_kept_whole() {
        let log = buffer(10);

        log.push(&"x".repeat(LINE_LIMIT));

        assert_eq!(log.lines(), ["x".repeat(LINE_LIMIT)]);
    }

    #[test]
    fn characters_that_would_act_on_a_screen_are_replaced() {
        let log = buffer(10);

        log.push("name\u{7}\u{1b}[31m\u{202e}evil\u{2066}\u{85}\tend");

        // The tab stays.
        assert_eq!(
            log.lines(),
            ["name\u{FFFD}\u{FFFD}[31m\u{FFFD}evil\u{FFFD}\u{FFFD}\tend"]
        );
    }

    #[test]
    fn characters_that_cannot_be_seen_are_replaced_too() {
        let log = buffer(10);

        // The direction marks, a zero-width space, the word joiner and the invisible
        // operators, the deprecated format characters, a byte order mark and an annotation mark.
        log.push(
            "a\u{200b}b\u{200e}c\u{200f}d\u{61c}e\u{2060}f\u{2064}g\u{206a}h\u{206f}i\u{feff}j\u{fff9}k\u{fffb}l",
        );

        assert_eq!(
            log.lines(),
            [
                "a\u{FFFD}b\u{FFFD}c\u{FFFD}d\u{FFFD}e\u{FFFD}f\u{FFFD}g\u{FFFD}h\u{FFFD}i\u{FFFD}j\u{FFFD}k\u{FFFD}l"
            ]
        );
    }

    #[test]
    fn the_joiners_that_emoji_and_scripts_need_stay() {
        let log = buffer(10);
        let kept = "family \u{1F468}\u{200D}\u{1F469}, a\u{200C}b, heart \u{2764}\u{FE0F}";

        log.push(kept);

        assert_eq!(log.lines(), [kept]);
    }

    #[test]
    fn a_pairing_code_is_removed_wherever_it_is_with_the_rest_of_the_event() {
        let log = buffer(10);
        let code = "BATON1:ABCDEFGHIJ0123456789-$%*+./:XYZ";

        log.push(&format!("joining {code} failed\n"));
        log.push(&format!("{code}\n"));
        log.push(&format!("two codes {code}, and baton1:abc123=def;\n"));
        log.push(&format!("ticket=\"{code}\"\n"));

        assert_eq!(
            log.lines(),
            [
                "joining BATON1:[removed]",
                "BATON1:[removed]",
                "two codes BATON1:[removed]",
                "ticket=\"BATON1:[removed]",
            ]
        );
    }

    #[test]
    fn a_code_written_in_groups_or_over_several_lines_is_removed_whole() {
        let log = buffer(10);

        // The parser of a ticket drops white space and ignores case.
        log.push("could not read baton1: ABCD EFGH\n  IJKL 1234\nlast line of the event\n");
        log.push("the next event\n");

        assert_eq!(
            log.lines(),
            ["could not read BATON1:[removed]", "the next event"]
        );
    }

    #[test]
    fn text_that_only_mentions_the_prefix_loses_the_rest_of_its_event() {
        let log = buffer(10);

        log.push("a code starts with BATON1: and ends\n");
        log.push("Zoë’s BATON1:é goes on\n");

        assert_eq!(
            log.lines(),
            [
                "a code starts with BATON1:[removed]",
                "Zoë’s BATON1:[removed]",
            ]
        );
    }

    #[test]
    fn what_says_who_a_device_belongs_to_or_where_it_is_is_taken_out() {
        let log = buffer(10);
        let key = "b0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecf";

        log.push(&format!(
            "device_id={key} at /Users/anna/Library/baton via 8.8.8.8:41641 and 192.168.1.4:41641"
        ));

        assert_eq!(
            log.lines(),
            [
                "device_id=b0b1b2b3b4… at /Users/~/Library/baton via public IPv4 (hidden):41641 and 192.168.1.4:41641"
            ]
        );
    }

    #[test]
    fn a_line_without_a_code_is_not_copied_for_nothing() {
        assert!(matches!(
            remove_codes("INFO baton_app: Baton started"),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn the_writer_keeps_an_event_when_it_is_done_with_it() {
        let log = buffer(10);
        let capture = Capture(log);

        {
            let mut writer = capture.make_writer();
            writer.write_all(b"2026-10-15T08:00:00Z  INFO ").unwrap();
            writer.write_all(b"baton_app: Baton started\n").unwrap();
            assert!(
                log.lines().is_empty(),
                "an event is kept whole, not in parts"
            );
        }
        // A writer that was given nothing keeps nothing.
        drop(capture.make_writer());

        assert_eq!(
            log.lines(),
            ["2026-10-15T08:00:00Z  INFO baton_app: Baton started"]
        );
    }

    #[test]
    fn a_log_with_a_panic_in_it_still_works() {
        let log = buffer(10);
        let poisoner = std::thread::spawn(move || {
            let _held = log.lines.lock().unwrap();
            panic!("a thread dies while it holds the lines");
        });
        assert!(poisoner.join().is_err());

        log.push("still here");

        assert_eq!(log.lines(), ["still here"]);
    }
}
