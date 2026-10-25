// SPDX-License-Identifier: Apache-2.0

//! What the thread that puts events to a window tells the log: how many events went, how many
//! were dropped and for which rule, and how long the Mac took to answer.
//!
//! A person whose tap did nothing reads one sentence on the other device, and the Mac that did not
//! act on it has to be able to say more than that: whether the picture was old or not known at
//! all, whether the window had the keyboard when the press came, and whether the Mac was slow. The
//! line holds counts, the names of the rules and times in milliseconds. It never says which events
//! they were, where on the window, or what was typed: that is the person's, and the log is pasted
//! into public reports.
//!
//! A count above [`EXACT`] is told as "more than that" for the same reason. A key goes down and
//! comes up as two events, and a password is typed one key at a time, so the exact number of
//! events, of the batches they came in or of the drops among them could be its length.
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

/// The most that a count is told as it is. Five events are two keys at most.
const EXACT: u32 = 5;

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
    /// Whether what is shared is a whole screen and not one window, which the line says.
    screen: bool,
    batches: u32,
    events: u32,
    posted: u32,
    /// Events that the rules let through and the platform did not take.
    refused: u32,
    /// Presses that went to a window that did not have the keyboard. An application may take the
    /// first press on such a window for the one that brings it forward, and not for the one that
    /// does something.
    unfocused: u32,
    /// Events that were refused because the thread did not keep up.
    lost: u32,
    /// Batches for which the platform could not say what is around the window.
    unread: u32,
    dropped: BTreeMap<Reason, u32>,
    /// The longest an event waited for the thread.
    waited: Duration,
    /// The longest the platform took to say what is around the window.
    reading: Duration,
    /// The longest the platform took to post an event.
    posting: Duration,
    /// The longest a picture that a dropped event named had been out of date.
    stale_for: Option<Duration>,
    /// The longest a picture that a press went through on had been out of date.
    pressed_on_old: Option<Duration>,
    /// The last frame that was named and is not kept.
    unknown: Option<Named>,
}

impl Journal {
    /// What is shared is a whole screen and not one window: the lines say so.
    pub(crate) fn of_display(&mut self, display: bool) {
        self.screen = display;
    }

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

    /// A button went down on the window, and the platform took it. `keyboard` is whether the window
    /// had the keyboard, and `old` how long the picture that the press named had been out of date,
    /// if it had been.
    pub(crate) fn pressed(&mut self, keyboard: bool, old: Option<Duration>) {
        if !keyboard {
            self.unfocused = self.unfocused.saturating_add(1);
        }
        if let Some(old) = old {
            self.pressed_on_old = Some(self.pressed_on_old.map_or(old, |most| most.max(old)));
        }
    }

    /// `events` were refused because the queue was full.
    pub(crate) fn lost(&mut self, events: usize) {
        self.lost = self
            .lost
            .saturating_add(u32::try_from(events).unwrap_or(u32::MAX));
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
        // What was lost is all there is to tell if the line before it was made after the last batch.
        if self.batches == 0 && self.lost == 0 {
            return None;
        }
        let trouble = !self.dropped.is_empty()
            || self.refused > 0
            || self.unread > 0
            || self.unfocused > 0
            || self.lost > 0;
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
            screen: self.screen,
            ..Self::default()
        };
        Some(line)
    }

    fn describe(&self) -> String {
        let noun = if self.screen { "screen" } else { "window" };
        let mut parts = Vec::new();
        if self.batches > 0 {
            parts.push(format!(
                "{} in {}",
                counted(self.events, "event", "events"),
                counted(self.batches, "batch", "batches")
            ));
        }
        if self.posted > 0 {
            parts.push(format!("{} put to the {noun}", few(self.posted)));
        }
        if self.unfocused > 0 {
            parts.push(format!(
                "{} that went to a window without the keyboard",
                counted(self.unfocused, "press", "presses")
            ));
        }
        if self.refused > 0 {
            // The Mac has not allowed the input, which is the first thing to rule out when a
            // session is tried, or, for a window, the key is one that the system keeps for itself.
            let why = if self.screen {
                "it may not post input"
            } else {
                "it may not post input, or the key is one that the system keeps"
            };
            parts.push(format!(
                "{} that the Mac would not take ({why})",
                counted(self.refused, "event", "events")
            ));
        }
        if self.lost > 0 {
            parts.push(format!(
                "{} lost because the queue was full",
                counted(self.lost, "event", "events")
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
                .map(|(reason, count)| format!("{reason:?} x{}", few(*count)))
                .collect();
            parts.push(format!("{} dropped ({})", few(total), reasons.join(", ")));
        }
        if self.unread > 0 {
            parts.push(format!(
                "the Mac could not say what is around the {noun} for {}",
                counted(self.unread, "batch", "batches")
            ));
        }
        let mut line = format!("{noun} input: {}", parts.join(", "));
        if self.batches > 0 {
            line = format!(
                "{line}; at most {} ms in the queue, {} ms for the Mac to say what is around the \
                 {noun} and {} ms to post",
                self.waited.as_millis(),
                self.reading.as_millis(),
                self.posting.as_millis(),
            );
        }
        if let Some(old) = self.stale_for {
            line = format!(
                "{line}; a picture that was named had been out of date for {} ms",
                old.as_millis()
            );
        }
        if let Some(old) = self.pressed_on_old {
            line = format!(
                "{line}; a press went through on a picture that had been out of date for {} ms",
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

/// A count as the log tells it: as it is up to [`EXACT`], and as `>5` above it.
fn few(count: u32) -> String {
    if count > EXACT {
        format!(">{EXACT}")
    } else {
        count.to_string()
    }
}

/// `1 event`, `2 events`, `>5 events`.
fn counted(count: u32, one: &str, many: &str) -> String {
    format!("{} {}", few(count), if count == 1 { one } else { many })
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
    fn a_press_that_went_to_a_window_without_the_keyboard_is_trouble_by_itself() {
        let mut journal = Journal::default();
        let start = Instant::now();
        news(&mut journal, 1);
        journal.posted(MS, true);
        assert!(journal.line(start, false).is_some());

        // Of two presses, only the one that went to a window without the keyboard is told.
        news(&mut journal, 2);
        journal.posted(MS, true);
        journal.pressed(false, None);
        journal.posted(MS, true);
        journal.pressed(true, None);
        assert_eq!(journal.line(start + EVERY / 2, false), None);
        assert_eq!(
            journal.line(start + EVERY, false).unwrap(),
            "window input: 2 events in 1 batch, 2 put to the window, 1 press that went to a window \
             without the keyboard; at most 3 ms in the queue, 12 ms for the Mac to say what is \
             around the window and 1 ms to post"
        );
    }

    #[test]
    fn a_press_that_went_through_on_an_old_picture_is_told_with_the_oldest_and_is_no_trouble() {
        let mut journal = Journal::default();
        let start = Instant::now();
        news(&mut journal, 1);
        assert!(journal.line(start, false).is_some());

        // The longest is told, and it is neither the first nor the last.
        news(&mut journal, 3);
        for old in [310, 640, 90] {
            journal.posted(MS, true);
            journal.pressed(true, Some(Duration::from_millis(old)));
        }
        // A press that went through is nothing to tell by itself.
        assert_eq!(journal.line(start + 10 * EVERY, false), None);
        assert_eq!(
            journal.line(start + 10 * EVERY, true).unwrap(),
            "window input: 3 events in 1 batch, 3 put to the window; at most 3 ms in the queue, \
             12 ms for the Mac to say what is around the window and 1 ms to post; a press went \
             through on a picture that had been out of date for 640 ms"
        );
    }

    #[test]
    fn the_age_of_the_pictures_comes_before_the_frame_that_is_not_kept() {
        let mut journal = Journal::default();
        news(&mut journal, 3);
        journal.posted(MS, true);
        journal.pressed(true, Some(Duration::from_millis(1_900)));
        journal.dropped(Reason::PictureOutOfDate, Some(stale(2_500)));
        journal.dropped(Reason::FrameUnknown, Some(unknown()));
        let line = journal.line(Instant::now(), false).unwrap();
        assert!(
            line.ends_with(
                "; a picture that was named had been out of date for 2500 ms; a press went \
                 through on a picture that had been out of date for 1900 ms; frame 9 was named \
                 and is not kept (newest kept: none, 0 kept)"
            ),
            "{line}"
        );
    }

    #[test]
    fn events_that_were_lost_are_trouble_by_themselves() {
        let mut journal = Journal::default();
        let start = Instant::now();
        news(&mut journal, 1);
        journal.posted(MS, true);
        assert!(journal.line(start, false).is_some());

        news(&mut journal, 1);
        journal.posted(MS, true);
        journal.lost(3);
        assert_eq!(journal.line(start + EVERY / 2, false), None);
        assert_eq!(
            journal.line(start + EVERY, false).unwrap(),
            "window input: 1 event in 1 batch, 1 put to the window, 3 events lost because the \
             queue was full; at most 3 ms in the queue, 12 ms for the Mac to say what is around \
             the window and 1 ms to post"
        );
    }

    #[test]
    fn what_was_lost_after_the_last_line_is_told_at_the_end_with_no_batch_to_go_with_it() {
        let mut journal = Journal::default();
        let start = Instant::now();
        news(&mut journal, 1);
        assert!(journal.line(start, false).is_some());

        journal.lost(1);
        assert_eq!(
            journal.line(start + MS, true).unwrap(),
            "window input: 1 event lost because the queue was full"
        );
        assert_eq!(journal.line(start + 2 * MS, true), None);
    }

    #[test]
    fn five_events_are_told_as_they_are_and_six_as_more_than_five() {
        let line = |count: u32| {
            let mut journal = Journal::default();
            for _ in 0..count {
                news(&mut journal, 1);
                journal.posted(MS, true);
            }
            journal.line(Instant::now(), true).unwrap()
        };
        assert!(
            line(5).starts_with("window input: 5 events in 5 batches, 5 put to the window; "),
            "{}",
            line(5)
        );
        assert!(
            line(6).starts_with("window input: >5 events in >5 batches, >5 put to the window; "),
            "{}",
            line(6)
        );
    }

    #[test]
    fn what_was_dropped_refused_or_lost_is_told_the_same_way() {
        let mut journal = Journal::default();
        news(&mut journal, 1);
        for _ in 0..6 {
            journal.dropped(Reason::Covered, None);
            journal.posted(MS, false);
            journal.pressed(false, None);
        }
        for _ in 0..5 {
            journal.dropped(Reason::Locked, None);
        }
        journal.lost(6);
        for _ in 0..6 {
            journal.batch(1, MS, MS, false);
        }
        let line = journal.line(Instant::now(), false).unwrap();
        for told in [
            ">5 presses that went to a window without the keyboard",
            ">5 events that the Mac would not take (it may not post input, or the key is one that \
             the system keeps)",
            ">5 events lost because the queue was full",
            ">5 dropped (Locked x5, Covered x>5)",
            "the Mac could not say what is around the window for >5 batches",
        ] {
            assert!(line.contains(told), "{told:?} in {line}");
        }
    }

    #[test]
    fn a_screen_is_told_as_a_screen_and_stays_one_in_the_lines_after_the_first() {
        let mut journal = Journal::default();
        journal.of_display(true);
        let start = Instant::now();
        news(&mut journal, 3);
        for _ in 0..3 {
            journal.posted(MS, true);
        }
        assert_eq!(
            journal.line(start, false).unwrap(),
            "screen input: 3 events in 1 batch, 3 put to the screen; at most 3 ms in the queue, \
             12 ms for the Mac to say what is around the screen and 1 ms to post"
        );

        // The next line is about the same screen, and a key that the system keeps is not among
        // the reasons for an event that the Mac would not take: a screen is not held back by the
        // list.
        journal.batch(1, MS, MS, false);
        journal.posted(MS, false);
        let line = journal.line(start + 10 * EVERY, false).unwrap();
        assert!(
            line.starts_with(
                "screen input: 1 event in 1 batch, 1 event that the Mac would not take (it may \
                 not post input), the Mac could not say what is around the screen for 1 batch; "
            ),
            "{line}"
        );
    }

    #[test]
    fn an_event_that_the_mac_would_not_take_is_told_with_both_things_to_rule_out_for_a_window() {
        let mut journal = Journal::default();
        news(&mut journal, 1);
        journal.posted(MS, false);
        let line = journal.line(Instant::now(), false).unwrap();
        assert!(
            line.starts_with(
                "window input: 1 event in 1 batch, 1 event that the Mac would not take (it may \
                 not post input, or the key is one that the system keeps); "
            ),
            "{line}"
        );
    }

    #[test]
    fn what_was_lost_in_several_goes_is_told_as_the_sum() {
        let mut journal = Journal::default();
        journal.lost(2);
        journal.lost(3);
        assert_eq!(
            journal.line(Instant::now(), true).unwrap(),
            "window input: 5 events lost because the queue was full"
        );

        // Neither go is more than five, and together they are.
        let mut journal = Journal::default();
        journal.lost(4);
        journal.lost(4);
        assert_eq!(
            journal.line(Instant::now(), true).unwrap(),
            "window input: >5 events lost because the queue was full"
        );
    }

    #[test]
    fn what_was_lost_is_told_as_more_than_five_whatever_the_number() {
        let mut journal = Journal::default();
        journal.lost(usize::MAX);
        assert_eq!(
            journal.line(Instant::now(), true).unwrap(),
            "window input: >5 events lost because the queue was full"
        );
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
