// SPDX-License-Identifier: Apache-2.0

//! What the thread that puts events to a window tells the log: how many events went, how many
//! were dropped and for which rule, and how long the Mac took to answer.
//!
//! A person whose tap did nothing reads one sentence on the other device, and the Mac that did not
//! act on it has to be able to say more than that: whether the picture was old or not known at
//! all, and whether the Mac was slow. The line holds counts, the names of the rules and times in
//! milliseconds. It never says which events they were, where on the window, or what was typed: that
//! is the person's, and the log is pasted into public reports.
//!
//! The first batch of a session makes a line, so that a log shows that events arrive at all, and
//! after that a batch that dropped something does, but not more often than [`EVERY`]. What is not
//! told yet is told with the next line, or at the end of the session.

use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use baton_stream::gate::{PictureReport, Reason};

/// The least time between two lines about trouble.
pub(crate) const EVERY: Duration = Duration::from_secs(2);

/// A frame that an event named and the source did not accept, and what the gate knew of it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Named {
    pub(crate) frame: u64,
    pub(crate) report: PictureReport,
}

/// What happened to the events since the last line.
#[derive(Debug, Default)]
pub(crate) struct Journal {
    /// When the last line was made.
    last: Option<Instant>,
    /// Whether the first line was made.
    begun: bool,
    batches: u32,
    events: u32,
    posted: u32,
    /// Events that the rules let through and the platform did not take.
    refused: u32,
    /// Batches for which the platform could not say what is around the window.
    unread: u32,
    dropped: BTreeMap<Reason, u32>,
    /// The longest an event waited for the thread.
    waited: Duration,
    /// The longest the platform took to say what is around the window.
    reading: Duration,
    /// The longest the platform took to post an event.
    posting: Duration,
    /// The longest a picture that was named had been out of date.
    stale_for: Option<Duration>,
    /// The last frame that was named and is not kept.
    unknown: Option<Named>,
}

impl Journal {
    /// A batch of `events` was served: it waited `waited` for the thread, and the platform took
    /// `reading` to say what is around the window, which it did if `read` is true.
    pub(crate) fn batch(&mut self, events: usize, waited: Duration, reading: Duration, read: bool) {
        self.batches = self.batches.saturating_add(1);
        self.events = self
            .events
            .saturating_add(u32::try_from(events).unwrap_or(u32::MAX));
        self.waited = self.waited.max(waited);
        self.reading = self.reading.max(reading);
        if !read {
            self.unread = self.unread.saturating_add(1);
        }
    }

    /// An event was put to the window, and the platform took `took` to do it. `taken` is whether
    /// it did.
    pub(crate) fn posted(&mut self, took: Duration, taken: bool) {
        self.posting = self.posting.max(took);
        if taken {
            self.posted = self.posted.saturating_add(1);
        } else {
            self.refused = self.refused.saturating_add(1);
        }
    }

    /// An event was dropped for `reason`. `named` is what the gate knew of the frame that it
    /// named, for the two reasons that are about the frame.
    pub(crate) fn dropped(&mut self, reason: Reason, named: Option<Named>) {
        let count = self.dropped.entry(reason).or_insert(0);
        *count = count.saturating_add(1);
        let Some(named) = named else { return };
        if named.report.known {
            if let Some(old) = named.report.out_of_date_for {
                self.stale_for = Some(self.stale_for.map_or(old, |most| most.max(old)));
            }
        } else {
            self.unknown = Some(named);
        }
    }

    /// The line to log now, if there is one. A `force` is the end of the session, which tells what
    /// is left whether or not anything went wrong.
    pub(crate) fn line(&mut self, now: Instant, force: bool) -> Option<String> {
        if self.batches == 0 {
            return None;
        }
        let trouble = !self.dropped.is_empty() || self.refused > 0 || self.unread > 0;
        let spaced = self
            .last
            .is_none_or(|last| now.saturating_duration_since(last) >= EVERY);
        if !(force || !self.begun || (trouble && spaced)) {
            return None;
        }
        let line = self.describe();
        *self = Self {
            last: Some(now),
            begun: true,
            ..Self::default()
        };
        Some(line)
    }

    fn describe(&self) -> String {
        let mut parts = vec![format!(
            "{} in {}",
            counted(self.events, "event", "events"),
            counted(self.batches, "batch", "batches")
        )];
        if self.posted > 0 {
            parts.push(format!("{} put to the window", self.posted));
        }
        if self.refused > 0 {
            parts.push(format!(
                "{} that the Mac would not take",
                counted(self.refused, "event", "events")
            ));
        }
        if !self.dropped.is_empty() {
            let total = self
                .dropped
                .values()
                .fold(0_u32, |sum, count| sum.saturating_add(*count));
            let reasons: Vec<String> = self
                .dropped
                .iter()
                .map(|(reason, count)| format!("{reason:?} x{count}"))
                .collect();
            parts.push(format!("{total} dropped ({})", reasons.join(", ")));
        }
        if self.unread > 0 {
            parts.push(format!(
                "the Mac could not say what is around the window for {}",
                counted(self.unread, "batch", "batches")
            ));
        }
        let mut line = format!(
            "window input: {}; at most {} ms in the queue, {} ms for the Mac to say what is around \
             the window and {} ms to post",
            parts.join(", "),
            self.waited.as_millis(),
            self.reading.as_millis(),
            self.posting.as_millis(),
        );
        if let Some(old) = self.stale_for {
            line = format!(
                "{line}; a picture that was named had been out of date for {} ms",
                old.as_millis()
            );
        }
        if let Some(Named { frame, report }) = self.unknown {
            let newest = report
                .newest
                .map_or_else(|| "none".to_owned(), |number| number.to_string());
            line = format!(
                "{line}; frame {frame} was named and is not kept (newest kept: {newest}, {} kept)",
                report.kept
            );
        }
        line
    }
}

/// `1 event`, `2 events`.
fn counted(count: u32, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn news(journal: &mut Journal, events: usize) {
        journal.batch(events, 3 * MS, 12 * MS, true);
    }

    fn stale(ms: u64) -> Named {
        Named {
            frame: 40,
            report: PictureReport {
                known: true,
                out_of_date_for: Some(Duration::from_millis(ms)),
                newest: Some(44),
                kept: 5,
            },
        }
    }

    fn unknown() -> Named {
        Named {
            frame: 9,
            report: PictureReport {
                known: false,
                out_of_date_for: None,
                newest: None,
                kept: 0,
            },
        }
    }

    #[test]
    fn nothing_is_told_before_a_batch() {
        let mut journal = Journal::default();
        assert_eq!(journal.line(Instant::now(), true), None);
    }

    #[test]
    fn the_first_batch_is_told_whatever_became_of_it() {
        let mut journal = Journal::default();
        news(&mut journal, 3);
        for _ in 0..3 {
            journal.posted(MS, true);
        }
        let line = journal.line(Instant::now(), false).unwrap();
        assert_eq!(
            line,
            "window input: 3 events in 1 batch, 3 put to the window; at most 3 ms in the queue, \
             12 ms for the Mac to say what is around the window and 1 ms to post"
        );
    }

    #[test]
    fn a_batch_that_went_well_is_not_told_after_the_first() {
        let mut journal = Journal::default();
        let start = Instant::now();
        news(&mut journal, 1);
        journal.posted(MS, true);
        assert!(journal.line(start, false).is_some());

        news(&mut journal, 1);
        journal.posted(MS, true);
        assert_eq!(journal.line(start + 10 * EVERY, false), None);
    }

    #[test]
    fn what_was_dropped_is_told_with_the_rule_and_how_old_the_picture_was() {
        let mut journal = Journal::default();
        let start = Instant::now();
        news(&mut journal, 4);
        // The longest is told, and it is neither the first nor the last.
        journal.dropped(Reason::PictureOutOfDate, Some(stale(730)));
        journal.dropped(Reason::PictureOutOfDate, Some(stale(1_200)));
        journal.dropped(Reason::PictureOutOfDate, Some(stale(900)));
        journal.dropped(Reason::NotPressed, None);
        assert_eq!(
            journal.line(start, false).unwrap(),
            "window input: 4 events in 1 batch, 4 dropped (PictureOutOfDate x3, NotPressed x1); \
             at most 3 ms in the queue, 12 ms for the Mac to say what is around the window and \
             0 ms to post; a picture that was named had been out of date for 1200 ms"
        );
    }

    #[test]
    fn a_frame_that_is_not_kept_is_told_with_what_the_gate_has() {
        let mut journal = Journal::default();
        news(&mut journal, 1);
        journal.dropped(Reason::FrameUnknown, Some(unknown()));
        assert_eq!(
            journal.line(Instant::now(), false).unwrap(),
            "window input: 1 event in 1 batch, 1 dropped (FrameUnknown x1); at most 3 ms in the \
             queue, 12 ms for the Mac to say what is around the window and 0 ms to post; frame 9 \
             was named and is not kept (newest kept: none, 0 kept)"
        );
    }

    #[test]
    fn a_frame_that_is_not_kept_is_told_after_the_age_of_a_picture_and_names_the_newest_kept() {
        let mut journal = Journal::default();
        news(&mut journal, 2);
        journal.dropped(Reason::PictureOutOfDate, Some(stale(2_500)));
        let mut other = unknown();
        other.report.newest = Some(44);
        other.report.kept = 5;
        journal.dropped(Reason::FrameUnknown, Some(other));
        let line = journal.line(Instant::now(), false).unwrap();
        assert!(
            line.ends_with(
                "; a picture that was named had been out of date for 2500 ms; frame 9 was named \
                 and is not kept (newest kept: 44, 5 kept)"
            ),
            "{line}"
        );
    }

    #[test]
    fn trouble_is_told_at_most_every_so_often_and_then_all_of_it_together() {
        let mut journal = Journal::default();
        let start = Instant::now();
        news(&mut journal, 1);
        assert!(journal.line(start, false).is_some());

        news(&mut journal, 2);
        journal.dropped(Reason::Covered, None);
        journal.dropped(Reason::Covered, None);
        assert_eq!(journal.line(start + EVERY / 2, false), None);

        news(&mut journal, 1);
        journal.dropped(Reason::Locked, None);
        let line = journal.line(start + EVERY, false).unwrap();
        // The rules are told in the order that the notices prefer them.
        assert!(
            line.contains("3 events in 2 batches, 3 dropped (Locked x1, Covered x2)"),
            "{line}"
        );

        // What was told is not told again.
        assert_eq!(journal.line(start + 10 * EVERY, true), None);
    }

    #[test]
    fn a_platform_that_cannot_say_what_is_around_the_window_is_told() {
        let mut journal = Journal::default();
        journal.batch(1, MS, 40 * MS, false);
        journal.posted(MS, false);
        let line = journal.line(Instant::now(), false).unwrap();
        assert!(
            line.contains("1 event that the Mac would not take"),
            "{line}"
        );
        assert!(
            line.contains("the Mac could not say what is around the window for 1 batch"),
            "{line}"
        );
        assert!(line.contains("40 ms for the Mac to say"), "{line}");
    }

    #[test]
    fn an_event_that_the_platform_would_not_take_is_trouble_by_itself() {
        let mut journal = Journal::default();
        let start = Instant::now();
        news(&mut journal, 1);
        journal.posted(MS, true);
        assert!(journal.line(start, false).is_some());

        news(&mut journal, 1);
        journal.posted(MS, false);
        assert_eq!(journal.line(start + EVERY / 2, false), None);
        let line = journal.line(start + EVERY, false).unwrap();
        assert!(
            line.contains("1 event that the Mac would not take"),
            "{line}"
        );
        assert!(!line.contains("put to the window"), "{line}");
    }

    #[test]
    fn a_platform_that_could_not_say_what_is_around_the_window_is_trouble_by_itself() {
        let mut journal = Journal::default();
        let start = Instant::now();
        news(&mut journal, 1);
        assert!(journal.line(start, false).is_some());

        journal.batch(1, MS, 40 * MS, false);
        assert_eq!(journal.line(start + EVERY / 2, false), None);
        let line = journal.line(start + EVERY, false).unwrap();
        assert!(
            line.contains("the Mac could not say what is around the window for 1 batch"),
            "{line}"
        );
        assert!(!line.contains("dropped"), "{line}");
    }

    #[test]
    fn the_longest_times_are_told_and_not_the_last_ones() {
        let mut journal = Journal::default();
        journal.batch(1, 40 * MS, 70 * MS, true);
        journal.posted(25 * MS, true);
        journal.batch(1, 5 * MS, 9 * MS, true);
        journal.posted(2 * MS, true);
        let line = journal.line(Instant::now(), false).unwrap();
        assert!(
            line.contains(
                "at most 40 ms in the queue, 70 ms for the Mac to say what is around the window \
                 and 25 ms to post"
            ),
            "{line}"
        );
    }

    #[test]
    fn the_end_of_the_session_tells_what_is_left() {
        let mut journal = Journal::default();
        let start = Instant::now();
        news(&mut journal, 1);
        assert!(journal.line(start, false).is_some());
        news(&mut journal, 4);
        journal.posted(MS, true);
        let line = journal.line(start + MS, true).unwrap();
        assert!(
            line.starts_with("window input: 4 events in 1 batch"),
            "{line}"
        );
    }
}
