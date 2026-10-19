// SPDX-License-Identifier: Apache-2.0

//! What a viewer does with the touches and the keys of its person before it sends them.
//!
//! A finger gives a position a hundred times a second or more, a keyboard gives keys that go down
//! and come up, and the keyboard on the screen gives text. The source takes at most
//! [`MAX_EVENTS_PER_SECOND`] events in a second, in batches of at most [`MAX_BATCH`], and a batch
//! that breaks a rule of [`crate::input`] ends the session. So what the person does goes through a
//! [`Queue`], which keeps to those limits by the viewer's own clock and gives only batches that
//! [`crate::input::validate_batch`] accepts. It has no clock of its own: it is told the time, like the
//! rest of this crate, and it sends nothing.
//!
//! The rules are those of ADR 0009, from the side of the viewer:
//!
//! - Nothing is sent unless the person at the source granted control with the offer. A viewer
//!   cannot ask for control and cannot turn it on later, so a queue is made for one or the other
//!   and stays so.
//! - A position is a position in the picture that the person was looking at, and the event names
//!   that picture by its frame number. The queue is told which frame is on the screen when an
//!   event is given, and refuses a position while none has been shown.
//! - A position outside the picture is not sent, unless a button is down: a drag that leaves the
//!   picture goes on at its edge, and so does its release. A position a hair outside is on the
//!   edge, so that a screen that rounds its layout does not turn a tap on the edge into nothing.
//! - A release is of something that was pressed. The queue keeps the buttons and the keys that it
//!   let down, refuses the release of any other, and refuses a key when more are held than a
//!   person has fingers ([`MAX_HELD_KEYS`]). [`Queue::release_all`] releases what is down, for a
//!   screen that is closed or that the person has left.
//! - Moves of a pointer that wait to be sent are joined into the latest one, so that a slow link
//!   delivers where the pointer is and not where it has been.
//! - Text is cut at the boundaries of characters into pieces of at most [`MAX_KEY_TEXT_UNITS`]
//!   UTF-16 units, each an event with no key, and what cannot be typed (a control character other
//!   than tab, line feed and carriage return) is left out.
//! - At most [`MAX_EVENTS_PER_SECOND`] events are taken in any second, and at most [`MAX_BATCH`]
//!   in a batch. What waits for room is bounded ([`MAX_PENDING`]): an event that starts something
//!   is refused when the queue is full, and a release never is.
//!
//! Typed text and key codes are secrets, so `Debug` prints neither.

use std::{
    collections::{BTreeSet, VecDeque},
    fmt,
    time::Duration,
};

use crate::{
    control::InputBatch,
    gate::MAX_HELD_KEYS,
    input::{
        Button, Event, InputError, MAX_BATCH, MAX_EVENTS_PER_SECOND, MAX_KEY_TEXT_UNITS, Modifiers,
        NO_KEY, PointerAction,
    },
};

/// Most events that wait to be sent. A person types and taps far slower than the rate, so a queue
/// that holds this many is a link that does not carry them, and what comes after is refused.
pub const MAX_PENDING: usize = 1024;

const SECOND: Duration = Duration::from_secs(1);

/// How far outside the picture a position may be and still count as on its edge, as a share of the
/// picture.
const EDGE: f32 = 0.001;

/// What the person did on the viewer, before it is known which picture they did it on. The queue
/// adds the frame.
#[derive(Clone, PartialEq)]
pub enum Input {
    /// A finger or a pointer.
    Pointer {
        /// Across the picture, from 0 to 1.
        x: f32,
        /// Down the picture, from 0 to 1.
        y: f32,
        /// The modifiers that are down.
        modifiers: Modifiers,
        /// What it did.
        action: PointerAction,
    },
    /// A key, or text that was typed with no key.
    Key {
        /// A USB HID usage of the keyboard page, or [`NO_KEY`] for text that came with no key.
        code: u16,
        /// Whether the key went down (true) or came up (false).
        down: bool,
        /// The modifiers that are down after this event.
        modifiers: Modifiers,
        /// Whether the system repeats a key that is held.
        repeat: bool,
        /// The text that the key produced. Text with no key is cut into pieces.
        text: Option<String>,
    },
}

// A key code and the text of a key are what a person types. They stay out of every log.
impl fmt::Debug for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pointer {
                x,
                y,
                modifiers,
                action,
            } => f
                .debug_struct("Pointer")
                .field("x", x)
                .field("y", y)
                .field("modifiers", modifiers)
                .field("action", action)
                .finish(),
            Self::Key {
                down, repeat, text, ..
            } => f
                .debug_struct("Key")
                .field("down", down)
                .field("repeat", repeat)
                .field("text_bytes", &text.as_ref().map(String::len))
                .finish_non_exhaustive(),
        }
    }
}

/// Why the queue did not take an event. None of these ends a session: the person did something
/// that has no meaning at the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    /// The person at the source did not grant control of the window.
    #[error("control of the window was not granted")]
    NotGranted,
    /// No picture has been shown yet, so a position has nothing to be in.
    #[error("no picture has been shown yet")]
    NoPicture,
    /// The position is outside the picture and no button is down.
    #[error("the position is outside the picture")]
    Outside,
    /// The release is of a button or a key that is not down.
    #[error("nothing is down that this releases")]
    NotPressed,
    /// More keys are held than a person has fingers.
    #[error("too many keys are held")]
    TooManyHeld,
    /// Too much waits to be sent.
    #[error("too much waits to be sent")]
    Full,
    /// A number or a length is beyond the limits of the format.
    #[error(transparent)]
    Invalid(#[from] InputError),
}

/// Where a button was last put, which is where [`Queue::release_all`] lifts it.
#[derive(Debug, Clone, Copy)]
struct Last {
    frame: u64,
    x: f32,
    y: f32,
    modifiers: Modifiers,
}

/// The events of a person's hands, on their way to the source.
pub struct Queue {
    granted: bool,
    /// What waits to be sent, oldest first.
    pending: VecDeque<Event>,
    /// When events were taken in the last second, and how many each time.
    taken: VecDeque<(Duration, usize)>,
    /// The sum of the counts in `taken`.
    in_last_second: usize,
    /// The buttons that the queue let down and that have not come up.
    buttons: BTreeSet<Button>,
    /// The keys that the queue let down and that have not come up.
    keys: BTreeSet<u16>,
    /// The last pointer event the queue took.
    last: Option<Last>,
}

// The keys that are down are key codes. They stay out of every log, so only how many are shown.
impl fmt::Debug for Queue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Queue")
            .field("granted", &self.granted)
            .field("pending", &self.pending.len())
            .field("buttons", &self.buttons.len())
            .field("keys", &self.keys.len())
            .finish_non_exhaustive()
    }
}

impl Queue {
    /// A queue for a session where the person at the source did (`granted`) or did not grant
    /// control. Nothing is taken in the second case.
    #[must_use]
    pub fn new(granted: bool) -> Self {
        Self {
            granted,
            pending: VecDeque::new(),
            taken: VecDeque::new(),
            in_last_second: 0,
            buttons: BTreeSet::new(),
            keys: BTreeSet::new(),
            last: None,
        }
    }

    /// Whether control was granted.
    #[must_use]
    pub fn granted(&self) -> bool {
        self.granted
    }

    /// How many events wait to be sent.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether nothing waits to be sent.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Takes what the person did. `shown` is the number of the frame on the screen, if one has
    /// been shown: a position is a position in that picture.
    ///
    /// # Errors
    ///
    /// The event has no meaning at the source, or breaks a limit of the format; see [`Refused`].
    /// The queue is unchanged then.
    pub fn push(&mut self, shown: Option<u64>, input: Input) -> Result<(), Refused> {
        if !self.granted {
            return Err(Refused::NotGranted);
        }
        match input {
            Input::Pointer {
                x,
                y,
                modifiers,
                action,
            } => self.pointer(shown, (x, y), modifiers, action),
            Input::Key {
                code,
                down,
                modifiers,
                repeat,
                text,
            } if code == NO_KEY => self.typed(down, modifiers, repeat, text),
            Input::Key {
                code,
                down,
                modifiers,
                repeat,
                text,
            } => self.key(code, down, modifiers, repeat, text),
        }
    }

    fn pointer(
        &mut self,
        shown: Option<u64>,
        (x, y): (f32, f32),
        modifiers: Modifiers,
        action: PointerAction,
    ) -> Result<(), Refused> {
        // A button that is down can always be lifted, whatever the screen shows by then.
        let frame = match (shown, self.last) {
            (Some(frame), _) => frame,
            (None, Some(last)) if matches!(action, PointerAction::Up(_)) => last.frame,
            (None, _) => return Err(Refused::NoPicture),
        };
        if !x.is_finite() || !y.is_finite() {
            return Err(InputError::NotFinite.into());
        }
        if let PointerAction::Up(button) = action
            && !self.buttons.contains(&button)
        {
            return Err(Refused::NotPressed);
        }
        let close = |at: f32| (-EDGE..=1.0 + EDGE).contains(&at);
        if !(close(x) && close(y)) && self.buttons.is_empty() {
            return Err(Refused::Outside);
        }
        let (x, y) = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
        let event = Event::Pointer {
            frame,
            x,
            y,
            modifiers,
            action,
        };
        event.validate()?;
        let joins = action == PointerAction::Move
            && matches!(
                self.pending.back(),
                Some(Event::Pointer {
                    action: PointerAction::Move,
                    modifiers: waiting,
                    ..
                }) if *waiting == modifiers
            );
        if joins {
            if let Some(waiting) = self.pending.back_mut() {
                *waiting = event;
            }
        } else if matches!(action, PointerAction::Up(_)) || self.has_room(1) {
            self.pending.push_back(event);
        } else {
            return Err(Refused::Full);
        }
        match action {
            PointerAction::Down(button) => {
                self.buttons.insert(button);
            }
            PointerAction::Up(button) => {
                self.buttons.remove(&button);
            }
            PointerAction::Move | PointerAction::Scroll { .. } => {}
        }
        self.last = Some(Last {
            frame,
            x,
            y,
            modifiers,
        });
        Ok(())
    }

    fn key(
        &mut self,
        code: u16,
        down: bool,
        modifiers: Modifiers,
        repeat: bool,
        text: Option<String>,
    ) -> Result<(), Refused> {
        // The key says what it is, and the text it produced is only attached. Characters that
        // cannot be typed are not a reason to lose the key.
        let text = text
            .map(|text| typable(&text))
            .filter(|text| !text.is_empty());
        let event = Event::Key {
            code,
            down,
            modifiers,
            repeat: down && repeat,
            text,
        };
        event.validate()?;
        let held = self.keys.contains(&code);
        if down {
            if !held && self.keys.len() >= MAX_HELD_KEYS {
                return Err(Refused::TooManyHeld);
            }
            if !self.has_room(1) {
                return Err(Refused::Full);
            }
            self.keys.insert(code);
        } else if held {
            self.keys.remove(&code);
        } else {
            return Err(Refused::NotPressed);
        }
        self.pending.push_back(event);
        Ok(())
    }

    fn typed(
        &mut self,
        down: bool,
        modifiers: Modifiers,
        repeat: bool,
        text: Option<String>,
    ) -> Result<(), Refused> {
        if !down || repeat {
            return Err(InputError::TextOnly.into());
        }
        let text = typable(&text.unwrap_or_default());
        let count = pieces(&text).count();
        if count == 0 {
            return Err(InputError::TextOnly.into());
        }
        // All of the text or none of it: half a paste is worse than none.
        if !self.has_room(count) {
            return Err(Refused::Full);
        }
        for piece in pieces(&text) {
            self.pending.push_back(Event::Key {
                code: NO_KEY,
                down: true,
                modifiers,
                repeat: false,
                text: Some(piece.to_owned()),
            });
        }
        Ok(())
    }

    fn has_room(&self, events: usize) -> bool {
        self.pending.len().saturating_add(events) <= MAX_PENDING
    }

    /// Queues the release of every button and every key that is down, to be sent after what waits
    /// now. A release is never refused for lack of room.
    ///
    /// A button comes up where it was last put, with the modifiers it had. The keys come up with
    /// none down, which is where the person's hands are.
    pub fn release_all(&mut self) {
        let buttons = std::mem::take(&mut self.buttons);
        if let Some(last) = self.last {
            for button in buttons {
                self.pending.push_back(Event::Pointer {
                    frame: last.frame,
                    x: last.x,
                    y: last.y,
                    modifiers: last.modifiers,
                    action: PointerAction::Up(button),
                });
            }
        }
        for code in std::mem::take(&mut self.keys) {
            self.pending.push_back(Event::Key {
                code,
                down: false,
                modifiers: Modifiers::NONE,
                repeat: false,
                text: None,
            });
        }
    }

    /// Lets go of the events that were taken more than a second before `now`.
    fn forget_old(&mut self, now: Duration) {
        while let Some(&(at, count)) = self.taken.front() {
            if now.saturating_sub(at) < SECOND {
                break;
            }
            self.taken.pop_front();
            self.in_last_second = self.in_last_second.saturating_sub(count);
        }
    }

    /// When the next batch can be taken: `None` if nothing waits, `now` if a batch can be taken at
    /// once, and the time at which the events of the oldest batch leave the last second otherwise.
    pub fn next_at(&mut self, now: Duration) -> Option<Duration> {
        if self.pending.is_empty() {
            return None;
        }
        self.forget_old(now);
        if self.in_last_second < MAX_EVENTS_PER_SECOND {
            return Some(now);
        }
        // A full second of events cannot be empty, but a queue that is wrong must not make a task
        // spin: a second from now is the latest it could be.
        Some(
            self.taken
                .front()
                .map_or(now + SECOND, |&(at, _)| at + SECOND),
        )
    }

    /// Takes the next batch, at most [`MAX_BATCH`] events and as many as the rate leaves room
    /// for, or `None` if nothing waits or the rate leaves no room. `now` is the viewer's clock,
    /// and does not go back.
    pub fn take(&mut self, now: Duration) -> Option<InputBatch> {
        self.forget_old(now);
        let room = MAX_EVENTS_PER_SECOND.saturating_sub(self.in_last_second);
        let count = self.pending.len().min(MAX_BATCH).min(room);
        if count == 0 {
            return None;
        }
        let events: Vec<Event> = self.pending.drain(..count).collect();
        self.taken.push_back((now, count));
        self.in_last_second += count;
        Some(InputBatch { events })
    }
}

/// The text without the characters that cannot be typed: a control character other than tab, line
/// feed and carriage return.
fn typable(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
        .collect()
}

/// The pieces that `text` is cut into: as long as can be, up to [`MAX_KEY_TEXT_UNITS`] UTF-16
/// units, and always whole characters.
fn pieces(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let mut units = 0;
        let mut end = 0;
        for c in rest.chars() {
            units += c.len_utf16();
            if units > MAX_KEY_TEXT_UNITS {
                break;
            }
            end += c.len_utf8();
        }
        // A character is at most two units, so a piece holds one at least.
        if end == 0 {
            return None;
        }
        let (piece, tail) = rest.split_at_checked(end)?;
        rest = tail;
        Some(piece)
    })
}

#[cfg(test)]
mod tests {
    use baton_proto::frame;
    use proptest::prelude::*;

    use super::*;
    use crate::{
        control::ViewerMessage,
        input::{MAX_KEY_CODE, validate_batch},
    };

    fn queue() -> Queue {
        Queue::new(true)
    }

    fn at(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    fn pointer(x: f32, y: f32, action: PointerAction) -> Input {
        Input::Pointer {
            x,
            y,
            modifiers: Modifiers::NONE,
            action,
        }
    }

    fn down(button: Button) -> PointerAction {
        PointerAction::Down(button)
    }

    fn up(button: Button) -> PointerAction {
        PointerAction::Up(button)
    }

    /// What a pointer event with no modifier comes out as.
    fn pointed(frame: u64, x: f32, y: f32, action: PointerAction) -> Event {
        Event::Pointer {
            frame,
            x,
            y,
            modifiers: Modifiers::NONE,
            action,
        }
    }

    fn key(code: u16, down: bool) -> Input {
        Input::Key {
            code,
            down,
            modifiers: Modifiers::NONE,
            repeat: false,
            text: None,
        }
    }

    /// What a key with no modifier and no text comes out as.
    fn pressed(code: u16, down: bool) -> Event {
        Event::Key {
            code,
            down,
            modifiers: Modifiers::NONE,
            repeat: false,
            text: None,
        }
    }

    fn typed(text: &str) -> Input {
        Input::Key {
            code: NO_KEY,
            down: true,
            modifiers: Modifiers::NONE,
            repeat: false,
            text: Some(text.to_owned()),
        }
    }

    /// Everything that can be taken, at times far enough apart that the rate leaves room.
    fn drain(queue: &mut Queue) -> Vec<Event> {
        drain_from(queue, at(0))
    }

    /// The same, from the time `now` on.
    fn drain_from(queue: &mut Queue, now: Duration) -> Vec<Event> {
        batches_from(queue, now)
            .into_iter()
            .flat_map(|batch| batch.events)
            .collect()
    }

    /// Every batch that can be taken, from the time `now` on.
    fn batches_from(queue: &mut Queue, mut now: Duration) -> Vec<InputBatch> {
        let mut all = Vec::new();
        let mut rounds = 0;
        while !queue.is_empty() {
            // A queue that the passing of time does not empty would otherwise never end the test.
            rounds += 1;
            assert!(rounds <= 1000, "time does not empty the queue");
            while let Some(batch) = queue.take(now) {
                all.push(batch);
            }
            now += Duration::from_secs(2);
        }
        all
    }

    /// The text of each event that has some, in order.
    fn texts(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Key { text, .. } => text.clone(),
                Event::Pointer { .. } => None,
            })
            .collect()
    }

    fn units(text: &str) -> usize {
        text.encode_utf16().count()
    }

    /// Fills the queue with text, one event each, until `left` places are left.
    fn fill(queue: &mut Queue, left: usize) {
        while queue.len() < MAX_PENDING - left {
            queue.push(Some(1), typed("a")).unwrap();
        }
    }

    #[test]
    fn nothing_is_taken_when_control_was_not_granted() {
        let mut queue = Queue::new(false);
        assert!(!queue.granted());
        let tap = pointer(0.5, 0.5, down(Button::Left));
        assert_eq!(queue.push(Some(1), tap), Err(Refused::NotGranted));
        assert_eq!(queue.push(Some(1), key(4, true)), Err(Refused::NotGranted));
        assert_eq!(queue.push(Some(1), typed("a")), Err(Refused::NotGranted));
        assert!(queue.is_empty());
        assert_eq!(queue.take(at(0)), None);
    }

    #[test]
    fn a_position_names_the_frame_that_was_on_the_screen() {
        let mut queue = queue();
        queue
            .push(Some(7), pointer(0.25, 0.75, PointerAction::Move))
            .unwrap();
        let batch = queue.take(at(0)).unwrap();
        assert_eq!(batch.events, [pointed(7, 0.25, 0.75, PointerAction::Move)]);
    }

    #[test]
    fn a_position_is_refused_while_no_picture_has_been_shown() {
        let mut queue = queue();
        let tap = pointer(0.5, 0.5, down(Button::Left));
        assert_eq!(queue.push(None, tap), Err(Refused::NoPicture));
        // A key names no picture.
        queue.push(None, key(4, true)).unwrap();
        queue.push(None, typed("a")).unwrap();
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn a_position_outside_the_picture_is_not_sent_while_no_button_is_down() {
        let mut queue = queue();
        for (x, y) in [(1.5, 0.5), (0.5, -0.2), (-1.0, 2.0), (1.01, 1.01)] {
            for action in [
                PointerAction::Move,
                down(Button::Left),
                PointerAction::Scroll { dx: 1.0, dy: 1.0 },
            ] {
                assert_eq!(
                    queue.push(Some(1), pointer(x, y, action)),
                    Err(Refused::Outside),
                    "{x} {y} {action:?}"
                );
            }
        }
        assert!(queue.is_empty());
        // A press that was refused holds nothing down.
        let lift = pointer(0.5, 0.5, up(Button::Left));
        assert_eq!(queue.push(Some(1), lift), Err(Refused::NotPressed));
    }

    #[test]
    fn a_position_that_is_not_a_number_is_refused_whatever_is_down() {
        for button_down in [false, true] {
            let mut queue = queue();
            if button_down {
                let press = pointer(0.5, 0.5, down(Button::Left));
                queue.push(Some(1), press).unwrap();
            }
            let before = queue.len();
            for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                for (x, y) in [(bad, 0.5), (0.5, bad)] {
                    assert_eq!(
                        queue.push(Some(1), pointer(x, y, PointerAction::Move)),
                        Err(Refused::Invalid(InputError::NotFinite)),
                        "{x} {y} {button_down}"
                    );
                }
            }
            assert_eq!(queue.len(), before);
        }
    }

    #[test]
    fn a_position_a_hair_outside_is_on_the_edge() {
        let mut queue = queue();
        queue
            .push(Some(1), pointer(1.0005, -0.0005, PointerAction::Move))
            .unwrap();
        assert_eq!(
            drain(&mut queue),
            [pointed(1, 1.0, 0.0, PointerAction::Move)]
        );
        assert_eq!(
            queue.push(Some(1), pointer(1.002, 0.5, PointerAction::Move)),
            Err(Refused::Outside)
        );
    }

    #[test]
    fn a_drag_that_leaves_the_picture_goes_on_at_its_edge_and_ends_there() {
        let mut queue = queue();
        let steps = [
            pointer(0.9, 0.5, down(Button::Left)),
            pointer(1.4, -0.3, PointerAction::Move),
            pointer(2.0, -0.3, up(Button::Left)),
        ];
        for step in steps {
            queue.push(Some(3), step).unwrap();
        }
        assert_eq!(
            drain(&mut queue),
            [
                pointed(3, 0.9, 0.5, down(Button::Left)),
                pointed(3, 1.0, 0.0, PointerAction::Move),
                pointed(3, 1.0, 0.0, up(Button::Left)),
            ]
        );
        // Nothing is down any more, so outside is outside again.
        assert_eq!(
            queue.push(Some(3), pointer(1.4, 0.5, PointerAction::Move)),
            Err(Refused::Outside)
        );
    }

    #[test]
    fn a_release_of_what_is_not_down_is_refused() {
        let mut queue = queue();
        let lift = |button| pointer(0.5, 0.5, up(button));
        assert_eq!(
            queue.push(Some(1), lift(Button::Left)),
            Err(Refused::NotPressed)
        );
        assert_eq!(queue.push(Some(1), key(4, false)), Err(Refused::NotPressed));
        // Another button is not this one.
        queue
            .push(Some(1), pointer(0.5, 0.5, down(Button::Right)))
            .unwrap();
        assert_eq!(
            queue.push(Some(1), lift(Button::Left)),
            Err(Refused::NotPressed)
        );
        queue.push(Some(1), lift(Button::Right)).unwrap();
        // And once only.
        assert_eq!(
            queue.push(Some(1), lift(Button::Right)),
            Err(Refused::NotPressed)
        );
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn a_button_can_be_lifted_when_no_picture_is_on_the_screen_any_more() {
        let mut queue = queue();
        queue
            .push(Some(5), pointer(0.2, 0.3, down(Button::Left)))
            .unwrap();
        // Only the release of what is down needs no picture.
        for action in [PointerAction::Move, down(Button::Right)] {
            assert_eq!(
                queue.push(None, pointer(0.2, 0.3, action)),
                Err(Refused::NoPicture)
            );
        }
        queue
            .push(None, pointer(0.2, 0.3, up(Button::Left)))
            .unwrap();
        assert_eq!(
            drain(&mut queue),
            [
                pointed(5, 0.2, 0.3, down(Button::Left)),
                pointed(5, 0.2, 0.3, up(Button::Left)),
            ]
        );
    }

    #[test]
    fn moves_that_wait_are_joined_into_the_latest() {
        let mut queue = queue();
        for step in 0_u8..10 {
            let at = f32::from(step) / 10.0;
            queue
                .push(Some(u64::from(step)), pointer(at, at, PointerAction::Move))
                .unwrap();
        }
        assert_eq!(queue.len(), 1);
        assert_eq!(
            drain(&mut queue),
            [pointed(9, 0.9, 0.9, PointerAction::Move)]
        );
    }

    #[test]
    fn a_move_is_joined_into_nothing_but_a_move_with_the_same_modifiers() {
        let mut queue = queue();
        let shifted = Input::Pointer {
            x: 0.5,
            y: 0.5,
            modifiers: Modifiers::SHIFT,
            action: PointerAction::Move,
        };
        let steps = [
            pointer(0.1, 0.1, PointerAction::Move),
            shifted,
            pointer(0.2, 0.2, down(Button::Left)),
            pointer(0.3, 0.3, PointerAction::Move),
            key(4, true),
            pointer(0.4, 0.4, PointerAction::Move),
            pointer(0.5, 0.5, PointerAction::Scroll { dx: 1.0, dy: 1.0 }),
            pointer(0.6, 0.6, PointerAction::Move),
        ];
        for step in steps {
            queue.push(Some(1), step).unwrap();
        }
        assert_eq!(queue.len(), 8);
    }

    #[test]
    fn a_key_goes_down_repeats_and_comes_up_with_its_modifiers() {
        let mut queue = queue();
        let shifted = |down, repeat| Input::Key {
            code: 4,
            down,
            modifiers: Modifiers::SHIFT,
            repeat,
            text: Some("A".to_owned()),
        };
        queue.push(None, shifted(true, false)).unwrap();
        queue.push(None, shifted(true, true)).unwrap();
        // The system repeats a key that is held, not one that came up.
        queue.push(None, shifted(false, true)).unwrap();
        let event = |down, repeat, text: Option<&str>| Event::Key {
            code: 4,
            down,
            modifiers: Modifiers::SHIFT,
            repeat,
            text: text.map(str::to_owned),
        };
        assert_eq!(
            drain(&mut queue),
            [
                event(true, false, Some("A")),
                event(true, true, Some("A")),
                event(false, false, Some("A")),
            ]
        );
    }

    #[test]
    fn no_more_keys_are_held_than_a_person_has_fingers() {
        let mut queue = queue();
        let held = u16::try_from(MAX_HELD_KEYS).unwrap();
        for code in 4..4 + held {
            queue.push(None, key(code, true)).unwrap();
        }
        assert_eq!(queue.push(None, key(100, true)), Err(Refused::TooManyHeld));
        // A key that is held already goes down again, which is a repeat and not another key.
        queue.push(None, key(5, true)).unwrap();
        // And a key that comes up makes room.
        queue.push(None, key(5, false)).unwrap();
        queue.push(None, key(100, true)).unwrap();
        // Text with no key holds nothing.
        queue.push(None, typed("hello")).unwrap();
    }

    #[test]
    fn text_with_no_key_goes_down_once_and_carries_text() {
        let mut queue = queue();
        let text_only = |down, repeat, text: Option<&str>| Input::Key {
            code: NO_KEY,
            down,
            modifiers: Modifiers::NONE,
            repeat,
            text: text.map(str::to_owned),
        };
        let only = Err(Refused::Invalid(InputError::TextOnly));
        assert_eq!(queue.push(None, text_only(false, false, Some("a"))), only);
        assert_eq!(queue.push(None, text_only(true, true, Some("a"))), only);
        assert_eq!(queue.push(None, text_only(true, false, None)), only);
        assert_eq!(queue.push(None, text_only(true, false, Some(""))), only);
        // Nothing that is left to type is nothing.
        assert_eq!(
            queue.push(None, text_only(true, false, Some("\u{7}\u{0}"))),
            only
        );
        assert!(queue.is_empty());
    }

    #[test]
    fn text_is_cut_into_pieces_of_twenty_units_at_the_boundaries_of_characters() {
        let mut queue = queue();
        let long = "x".repeat(45);
        // Eleven pairs of units: ten of them fill a piece.
        let wide = "😀".repeat(11);
        // Nineteen units and a pair: the pair starts the next piece.
        let edge = format!("{}😀", "a".repeat(19));
        for text in [long.as_str(), wide.as_str(), edge.as_str()] {
            queue.push(None, typed(text)).unwrap();
        }
        let pieces = texts(&drain(&mut queue));
        let lengths: Vec<usize> = pieces.iter().map(|piece| units(piece)).collect();
        assert_eq!(lengths, [20, 20, 5, 20, 2, 19, 2]);
        assert_eq!(pieces[0..3].concat(), long);
        assert_eq!(pieces[3..5].concat(), wide);
        assert_eq!(pieces[5..7].concat(), edge);
    }

    #[test]
    fn the_modifiers_of_text_with_no_key_go_with_each_piece() {
        let mut queue = queue();
        let input = Input::Key {
            code: NO_KEY,
            down: true,
            modifiers: Modifiers::OPTION,
            repeat: false,
            text: Some("y".repeat(25)),
        };
        queue.push(None, input).unwrap();
        let events = drain(&mut queue);
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| matches!(
            event,
            Event::Key {
                code: NO_KEY,
                down: true,
                modifiers: Modifiers::OPTION,
                repeat: false,
                ..
            }
        )));
    }

    #[test]
    fn characters_that_cannot_be_typed_are_left_out_of_text_but_tabs_and_line_breaks_stay() {
        let mut queue = queue();
        queue
            .push(None, typed("a\u{0}b\u{7}c\td\r\ne\u{1b}\u{7f}f"))
            .unwrap();
        assert_eq!(texts(&drain(&mut queue)), ["abc\td\r\nef"]);
    }

    #[test]
    fn a_key_is_sent_when_the_text_it_produced_cannot_be_typed() {
        let mut queue = queue();
        let backspace = Input::Key {
            code: 0x2A,
            down: true,
            modifiers: Modifiers::NONE,
            repeat: false,
            text: Some("\u{8}".to_owned()),
        };
        let enter = Input::Key {
            code: 0x28,
            down: true,
            modifiers: Modifiers::NONE,
            repeat: false,
            text: Some("\n".to_owned()),
        };
        queue.push(None, backspace).unwrap();
        queue.push(None, enter).unwrap();
        let events = drain(&mut queue);
        assert_eq!(events[0], pressed(0x2A, true));
        assert_eq!(
            events[1],
            Event::Key {
                code: 0x28,
                down: true,
                modifiers: Modifiers::NONE,
                repeat: false,
                text: Some("\n".to_owned()),
            }
        );
    }

    #[test]
    fn a_key_beyond_the_keyboard_page_or_with_too_much_text_is_refused() {
        let mut queue = queue();
        assert_eq!(
            queue.push(None, key(MAX_KEY_CODE + 1, true)),
            Err(Refused::Invalid(InputError::KeyCode))
        );
        let long = Input::Key {
            code: 4,
            down: true,
            modifiers: Modifiers::NONE,
            repeat: false,
            text: Some("a".repeat(MAX_KEY_TEXT_UNITS + 1)),
        };
        assert_eq!(
            queue.push(None, long),
            Err(Refused::Invalid(InputError::TextTooLong))
        );
        // The last key of the page is a key.
        queue.push(None, key(MAX_KEY_CODE, true)).unwrap();
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn a_scroll_beyond_the_limit_is_refused() {
        let mut queue = queue();
        let step = pointer(
            0.5,
            0.5,
            PointerAction::Scroll {
                dx: 0.0,
                dy: 10_001.0,
            },
        );
        assert_eq!(
            queue.push(Some(1), step),
            Err(Refused::Invalid(InputError::ScrollTooLarge))
        );
        assert!(queue.is_empty());
    }

    #[test]
    fn a_batch_has_at_most_sixty_four_events_and_the_events_keep_their_order() {
        let mut queue = queue();
        for number in 0..150 {
            queue.push(None, typed(&number.to_string())).unwrap();
        }
        let sizes: Vec<usize> = std::iter::from_fn(|| queue.take(at(0)))
            .map(|batch| batch.events.len())
            .collect();
        assert_eq!(sizes, [64, 64, 22]);
        assert!(queue.is_empty());

        for number in 0..150 {
            queue.push(None, typed(&number.to_string())).unwrap();
        }
        let expected: Vec<String> = (0..150).map(|number: u32| number.to_string()).collect();
        assert_eq!(texts(&drain_from(&mut queue, at(5000))), expected);
    }

    #[test]
    fn at_most_the_rate_is_taken_in_any_second_by_the_clock_that_was_given() {
        let mut queue = queue();
        for _ in 0..700 {
            queue.push(None, typed("a")).unwrap();
        }
        let mut take = |now: u64| -> usize {
            std::iter::from_fn(|| queue.take(at(now)))
                .map(|batch| batch.events.len())
                .sum()
        };
        assert_eq!(take(0), MAX_EVENTS_PER_SECOND);
        assert_eq!(take(500), 0);
        assert_eq!(take(999), 0);
        // The events of time zero have left the second.
        assert_eq!(take(1000), MAX_EVENTS_PER_SECOND);
        assert_eq!(take(1999), 0);
        assert_eq!(take(2000), 700 - 2 * MAX_EVENTS_PER_SECOND);
        assert_eq!(take(3000), 0);
    }

    #[test]
    fn the_second_slides_with_the_events_that_were_taken() {
        let mut queue = queue();
        let mut take = |now: u64, pushed: usize| -> usize {
            for _ in 0..pushed {
                queue.push(None, typed("a")).unwrap();
            }
            std::iter::from_fn(|| queue.take(at(now)))
                .map(|batch| batch.events.len())
                .sum()
        };
        assert_eq!(take(0, 100), 100);
        assert_eq!(take(600, 100), 100);
        // Room for forty more, of the hundred that wait.
        assert_eq!(take(900, 100), 40);
        assert_eq!(take(999, 0), 0);
        // The first hundred have left the second, and the 140 that were taken since have not.
        assert_eq!(take(1000, 0), 60);
        // The hundred of 600 are still in the second, and a hundred and forty with them.
        assert_eq!(take(1599, 100), 40);
        // And a moment later they have gone too, which leaves room for the rest.
        assert_eq!(take(1600, 0), 60);
    }

    #[test]
    fn the_time_for_the_next_batch_is_when_the_oldest_leave_the_second() {
        let mut queue = queue();
        assert_eq!(queue.next_at(at(0)), None);
        for _ in 0..500 {
            queue.push(None, typed("a")).unwrap();
        }
        assert_eq!(queue.next_at(at(0)), Some(at(0)));
        // 64 at 100 ms, 64 at 200 ms, 64 at 300 ms and 48 at 400 ms are the whole second.
        for (now, count) in [(100, 64), (200, 64), (300, 64), (400, 48)] {
            let batch = queue.take(at(now)).unwrap();
            assert_eq!(batch.events.len(), count);
        }
        assert_eq!(queue.take(at(450)), None);
        assert_eq!(queue.next_at(at(450)), Some(at(1100)));
        assert_eq!(queue.take(at(1099)), None);
        // At 1100 the first batch has left the second, and its 64 events are room again.
        assert_eq!(queue.next_at(at(1100)), Some(at(1100)));
        assert_eq!(queue.take(at(1100)).unwrap().events.len(), 64);
        assert_eq!(queue.next_at(at(1100)), Some(at(1200)));
    }

    #[test]
    fn a_second_with_one_place_left_has_room_for_a_batch_of_one() {
        let mut queue = queue();
        for _ in 0..239 {
            queue.push(None, typed("a")).unwrap();
        }
        for (now, count) in [(0, 64), (10, 64), (20, 64), (30, 47)] {
            assert_eq!(queue.take(at(now)).unwrap().events.len(), count);
        }
        for _ in 0..60 {
            queue.push(None, typed("b")).unwrap();
        }
        // 239 events were taken in the last second, so one more can be.
        assert_eq!(queue.next_at(at(40)), Some(at(40)));
        assert_eq!(queue.take(at(40)).unwrap().events.len(), 1);
        // Now the second is full until the first batch has left it.
        assert_eq!(queue.next_at(at(40)), Some(at(1000)));
        assert_eq!(queue.take(at(999)), None);
        assert_eq!(queue.take(at(1000)).unwrap().events.len(), 59);
    }

    #[test]
    fn a_queue_that_is_full_refuses_what_starts_something_but_not_a_release() {
        let mut queue = queue();
        queue
            .push(Some(1), pointer(0.5, 0.5, down(Button::Left)))
            .unwrap();
        queue.push(Some(1), key(4, true)).unwrap();
        fill(&mut queue, 0);
        assert_eq!(queue.len(), MAX_PENDING);
        let refused = [
            pointer(0.5, 0.5, PointerAction::Move),
            pointer(0.5, 0.5, down(Button::Right)),
            pointer(0.5, 0.5, PointerAction::Scroll { dx: 0.0, dy: 1.0 }),
            key(5, true),
            key(4, true),
            typed("a"),
        ];
        for input in refused {
            assert_eq!(
                queue.push(Some(1), input.clone()),
                Err(Refused::Full),
                "{input:?}"
            );
        }
        queue
            .push(Some(1), pointer(0.5, 0.5, up(Button::Left)))
            .unwrap();
        queue.push(Some(1), key(4, false)).unwrap();
        assert_eq!(queue.len(), MAX_PENDING + 2);
    }

    #[test]
    fn text_that_does_not_fit_is_refused_whole() {
        let mut queue = queue();
        fill(&mut queue, 2);
        // Three pieces, and room for two.
        assert_eq!(queue.push(None, typed(&"b".repeat(41))), Err(Refused::Full));
        assert_eq!(queue.len(), MAX_PENDING - 2);
        queue.push(None, typed(&"b".repeat(40))).unwrap();
        assert_eq!(queue.len(), MAX_PENDING);
    }

    #[test]
    fn release_all_lifts_the_buttons_where_they_were_and_releases_the_keys() {
        let mut queue = queue();
        let shifted = |x, y, action| Input::Pointer {
            x,
            y,
            modifiers: Modifiers::SHIFT,
            action,
        };
        queue
            .push(Some(9), shifted(0.3, 0.4, down(Button::Right)))
            .unwrap();
        queue
            .push(Some(9), shifted(0.3, 0.4, down(Button::Left)))
            .unwrap();
        queue
            .push(Some(9), shifted(0.6, 0.7, PointerAction::Move))
            .unwrap();
        queue.push(None, key(0xE1, true)).unwrap();
        queue.push(None, key(4, true)).unwrap();
        queue.push(None, typed("x")).unwrap();
        queue.release_all();
        let events = drain(&mut queue);
        let lifted = |button| Event::Pointer {
            frame: 9,
            x: 0.6,
            y: 0.7,
            modifiers: Modifiers::SHIFT,
            action: up(button),
        };
        assert_eq!(
            &events[events.len() - 4..],
            [
                lifted(Button::Left),
                lifted(Button::Right),
                pressed(4, false),
                pressed(0xE1, false),
            ]
        );
        assert_eq!(events.len(), 3 + 3 + 4);
    }

    #[test]
    fn release_all_does_nothing_when_nothing_is_down_and_leaves_nothing_down() {
        let mut queue = queue();
        queue.release_all();
        assert!(queue.is_empty());

        queue
            .push(Some(1), pointer(0.5, 0.5, down(Button::Left)))
            .unwrap();
        queue.push(None, key(4, true)).unwrap();
        queue.release_all();
        let before = queue.len();
        queue.release_all();
        assert_eq!(queue.len(), before);
        assert_eq!(
            queue.push(Some(1), pointer(0.5, 0.5, up(Button::Left))),
            Err(Refused::NotPressed)
        );
        assert_eq!(queue.push(None, key(4, false)), Err(Refused::NotPressed));
        // Nothing is down, so a drag that left the picture is not one any more.
        assert_eq!(
            queue.push(Some(1), pointer(2.0, 2.0, PointerAction::Move)),
            Err(Refused::Outside)
        );
    }

    #[test]
    fn a_release_is_never_refused_for_the_rate_and_waits_its_turn() {
        let mut queue = queue();
        queue
            .push(Some(1), pointer(0.5, 0.5, down(Button::Left)))
            .unwrap();
        for _ in 0..MAX_EVENTS_PER_SECOND {
            queue.push(None, typed("a")).unwrap();
        }
        queue.release_all();
        let mut taken = 0;
        while let Some(batch) = queue.take(at(0)) {
            taken += batch.events.len();
        }
        assert_eq!(taken, MAX_EVENTS_PER_SECOND);
        // The release is the first thing sent once there is room.
        let next = queue.take(at(1000)).unwrap();
        assert_eq!(next.events.len(), 2);
        assert!(matches!(
            next.events[1],
            Event::Pointer {
                action: PointerAction::Up(Button::Left),
                ..
            }
        ));
    }

    #[test]
    fn typed_text_and_key_codes_stay_out_of_the_debug_output() {
        let secret = Input::Key {
            code: 0x1F,
            down: true,
            modifiers: Modifiers::NONE,
            repeat: false,
            text: Some("hunter2".to_owned()),
        };
        let mut queue = queue();
        queue.push(None, secret.clone()).unwrap();
        queue.push(None, typed("swordfish")).unwrap();
        let shown = format!("{secret:?} {queue:?}");
        for hidden in ["hunter2", "swordfish", "31", "0x1f", "0x1F"] {
            assert!(!shown.contains(hidden), "{hidden} in {shown}");
        }
    }

    // What the queue gives is always sound, whatever it is given.

    fn modifiers() -> impl Strategy<Value = Modifiers> {
        (0_u8..32).prop_map(|bits| Modifiers::from_bits(bits).unwrap())
    }

    fn button() -> impl Strategy<Value = Button> {
        prop_oneof![
            Just(Button::Left),
            Just(Button::Right),
            Just(Button::Middle)
        ]
    }

    fn action() -> impl Strategy<Value = PointerAction> {
        prop_oneof![
            4 => Just(PointerAction::Move),
            2 => button().prop_map(PointerAction::Down),
            2 => button().prop_map(PointerAction::Up),
            1 => (-20_000.0_f32..20_000.0, -20_000.0_f32..20_000.0)
                .prop_map(|(dx, dy)| PointerAction::Scroll { dx, dy }),
        ]
    }

    fn coordinate() -> impl Strategy<Value = f32> {
        prop_oneof![
            8 => -0.5_f32..1.5,
            1 => Just(f32::NAN),
            1 => Just(f32::INFINITY),
            1 => Just(0.0),
            1 => Just(1.0),
        ]
    }

    fn input() -> impl Strategy<Value = Input> {
        let pointer = (coordinate(), coordinate(), modifiers(), action()).prop_map(
            |(x, y, modifiers, action)| Input::Pointer {
                x,
                y,
                modifiers,
                action,
            },
        );
        let text = proptest::option::of("[a-z \t\r\n\u{0}-\u{1f}\u{7f}é😀]{0,45}");
        // Few codes, so that the same key goes down and comes up, and some beyond the page.
        let code = prop_oneof![9 => 0_u16..0x20, 1 => 0xE8_u16..0x200];
        let key = (code, any::<bool>(), modifiers(), any::<bool>(), text).prop_map(
            |(code, down, modifiers, repeat, text)| Input::Key {
                code,
                down,
                modifiers,
                repeat,
                text,
            },
        );
        prop_oneof![3 => pointer, 2 => key]
    }

    #[derive(Debug, Clone)]
    enum Step {
        Push(Option<u64>, Input),
        Wait(u64),
        Release,
    }

    fn step() -> impl Strategy<Value = Step> {
        prop_oneof![
            10 => (proptest::option::of(0_u64..4), input())
                .prop_map(|(shown, input)| Step::Push(shown, input)),
            4 => (0_u64..700).prop_map(Step::Wait),
            1 => Just(Step::Release),
        ]
    }

    /// Checks one batch, and gives the time and size of it for the check of the rate.
    fn sound(batch: &InputBatch) -> Result<(), TestCaseError> {
        prop_assert!(!batch.events.is_empty() && batch.events.len() <= MAX_BATCH);
        prop_assert_eq!(validate_batch(&batch.events), Ok(()));
        // As a message it passes the checks of the wire.
        prop_assert!(frame::encode(&ViewerMessage::Input(batch.clone())).is_ok());
        Ok(())
    }

    /// Everything that went down came up, in a stream of events that a source could take.
    fn balanced(events: &[Event]) -> Result<(), TestCaseError> {
        let mut buttons = BTreeSet::new();
        let mut keys = BTreeSet::new();
        for event in events {
            match event {
                Event::Pointer {
                    action: PointerAction::Down(button),
                    ..
                } => {
                    buttons.insert(*button);
                }
                Event::Pointer {
                    action: PointerAction::Up(button),
                    ..
                } => prop_assert!(buttons.remove(button), "{event:?}"),
                Event::Key { code, down, .. } if *code != NO_KEY => {
                    if *down {
                        keys.insert(*code);
                    } else {
                        prop_assert!(keys.remove(code), "{event:?}");
                    }
                }
                _ => {}
            }
        }
        prop_assert!(buttons.is_empty() && keys.is_empty());
        Ok(())
    }

    proptest! {
        #[test]
        fn whatever_the_person_does_what_is_taken_is_sound_and_within_the_rate(
            steps in proptest::collection::vec(step(), 0..300)
        ) {
            let mut queue = Queue::new(true);
            let mut now = Duration::ZERO;
            let mut takes: Vec<(Duration, usize)> = Vec::new();
            let mut all: Vec<Event> = Vec::new();
            for step in steps {
                match step {
                    Step::Push(shown, input) => {
                        let before = queue.len();
                        let pushed = queue.push(shown, input);
                        // A refused event leaves no trace.
                        if pushed.is_err() {
                            prop_assert_eq!(queue.len(), before);
                        }
                    }
                    Step::Release => queue.release_all(),
                    Step::Wait(ms) => {
                        now += Duration::from_millis(ms);
                        while let Some(batch) = queue.take(now) {
                            sound(&batch)?;
                            takes.push((now, batch.events.len()));
                            let in_second: usize = takes
                                .iter()
                                .filter(|(at, _)| now.saturating_sub(*at) < SECOND)
                                .map(|(_, count)| count)
                                .sum();
                            prop_assert!(in_second <= MAX_EVENTS_PER_SECOND);
                            all.extend(batch.events);
                        }
                    }
                }
                // What starts something is bounded, and what ends it is bounded by what is down.
                prop_assert!(queue.len() <= MAX_PENDING + MAX_HELD_KEYS + 3);
            }
            queue.release_all();
            all.extend(drain_from(&mut queue, now));
            balanced(&all)?;
        }

        #[test]
        fn any_text_is_cut_into_whole_characters_of_at_most_twenty_units(text in any::<String>()) {
            let clean = typable(&text);
            let cut: Vec<&str> = pieces(&clean).collect();
            prop_assert_eq!(cut.concat(), clean.as_str());
            for piece in cut {
                prop_assert!(!piece.is_empty() && units(piece) <= MAX_KEY_TEXT_UNITS);
            }
        }

        #[test]
        fn any_text_that_is_taken_is_a_batch_that_a_source_accepts(text in any::<String>()) {
            let mut queue = Queue::new(true);
            if queue.push(None, typed(&text)).is_ok() {
                for batch in batches_from(&mut queue, Duration::ZERO) {
                    sound(&batch)?;
                }
            }
        }
    }
}
