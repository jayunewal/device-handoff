// SPDX-License-Identifier: Apache-2.0

//! What a viewer sends when it was granted control: where it tapped, what it typed.
//!
//! Nothing a viewer sends is trusted. [`validate_batch`] checks every number and length before
//! anything is used, and a batch that fails is a violation that ends the session. What is left
//! of the checks (is the point inside the shared window, is anything in front of it) depends on
//! what the Mac is doing at that moment, and is the job of the `gate` module, which is added next.
//!
//! A key event carries no frame number and no position: it is a key code, the modifiers that
//! are down and, optionally, the text that the key produced. A pointer event names the frame of
//! video that the person was looking at, because the position is a position in that picture.
//!
//! Typed text and key codes are secrets (a password is typed one key at a time), so `Debug`
//! does not print them: a key event shows whether it went down and how long its text is.

use std::{fmt, ops::BitOr};

use serde::{Deserialize, Serialize};

/// Most events in one batch.
pub const MAX_BATCH: usize = 64;

/// Most events a viewer may send in a second.
pub const MAX_EVENTS_PER_SECOND: usize = 240;

/// Longest text that a key can carry, in bytes.
pub const MAX_KEY_TEXT_BYTES: usize = 32;

/// Largest scroll step in either direction, in points.
pub const MAX_SCROLL: f32 = 10_000.0;

/// A field that a viewer sent that does not satisfy the limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InputError {
    /// A number is not finite.
    #[error("a number is not finite")]
    NotFinite,
    /// A position is outside the picture.
    #[error("a position is outside 0 to 1")]
    OutOfRange,
    /// A scroll step is larger than allowed.
    #[error("a scroll step is larger than {MAX_SCROLL}")]
    ScrollTooLarge,
    /// The text of a key is too long.
    #[error("the text of a key is longer than {MAX_KEY_TEXT_BYTES} bytes")]
    TextTooLong,
    /// The text of a key holds a character that cannot be typed.
    #[error("the text of a key has a control character")]
    TextHasControl,
    /// The modifiers have bits that are not defined.
    #[error("the modifiers have bits that are not defined")]
    Modifiers,
    /// A batch has too many events.
    #[error("a batch has more than {MAX_BATCH} events")]
    TooManyEvents,
}

/// A button of a pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Button {
    /// The primary button, or a touch.
    Left,
    /// The secondary button.
    Right,
    /// The middle button.
    Middle,
}

/// What a pointer did.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PointerAction {
    /// It moved, with a button held down or not.
    Move,
    /// A button went down.
    Down(Button),
    /// A button came up.
    Up(Button),
    /// A scroll, in points.
    Scroll {
        /// Along the x axis.
        dx: f32,
        /// Along the y axis.
        dy: f32,
    },
}

/// Which modifier keys are down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Modifiers(u8);

impl Modifiers {
    /// None.
    pub const NONE: Self = Self(0);
    /// Shift.
    pub const SHIFT: Self = Self(0b0_0001);
    /// Control.
    pub const CONTROL: Self = Self(0b0_0010);
    /// Option.
    pub const OPTION: Self = Self(0b0_0100);
    /// Command.
    pub const COMMAND: Self = Self(0b0_1000);
    /// Caps lock.
    pub const CAPS_LOCK: Self = Self(0b1_0000);

    const ALL: u8 = 0b1_1111;

    /// Reads modifiers from a byte, or refuses a byte with bits that are not defined.
    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits & !Self::ALL == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    /// The byte.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether every modifier of `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for Modifiers {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// One thing the person did on the viewer.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum Event {
    /// A pointer, a finger or a mouse.
    Pointer {
        /// The frame of video that the person was looking at.
        frame: u64,
        /// Where, from 0 to 1 across the picture of that frame.
        x: f32,
        /// Where, from 0 to 1 down the picture of that frame.
        y: f32,
        /// The modifiers that are down.
        modifiers: Modifiers,
        /// What it did.
        action: PointerAction,
    },
    /// A key.
    Key {
        /// The virtual key code.
        code: u16,
        /// Whether the key went down (true) or came up (false).
        down: bool,
        /// The modifiers that are down after this event.
        modifiers: Modifiers,
        /// Whether the system repeats a key that is held.
        repeat: bool,
        /// The text that the key produced, which the Mac attaches to the event.
        text: Option<String>,
    },
}

// A key code and the text of a key are what a person types. They stay out of every log.
impl fmt::Debug for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pointer {
                frame,
                x,
                y,
                modifiers,
                action,
            } => f
                .debug_struct("Pointer")
                .field("frame", frame)
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

impl Event {
    /// Checks every number and length.
    pub fn validate(&self) -> Result<(), InputError> {
        match self {
            Self::Pointer {
                x,
                y,
                modifiers,
                action,
                ..
            } => {
                if !x.is_finite() || !y.is_finite() {
                    return Err(InputError::NotFinite);
                }
                if !(0.0..=1.0).contains(x) || !(0.0..=1.0).contains(y) {
                    return Err(InputError::OutOfRange);
                }
                check_modifiers(*modifiers)?;
                if let PointerAction::Scroll { dx, dy } = action {
                    if !dx.is_finite() || !dy.is_finite() {
                        return Err(InputError::NotFinite);
                    }
                    if dx.abs() > MAX_SCROLL || dy.abs() > MAX_SCROLL {
                        return Err(InputError::ScrollTooLarge);
                    }
                }
                Ok(())
            }
            Self::Key {
                modifiers, text, ..
            } => {
                check_modifiers(*modifiers)?;
                let Some(text) = text else {
                    return Ok(());
                };
                if text.len() > MAX_KEY_TEXT_BYTES {
                    return Err(InputError::TextTooLong);
                }
                // A tab, a line feed and a carriage return are what the Tab and Return keys
                // produce. Every other control character is not typed.
                if text
                    .chars()
                    .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
                {
                    return Err(InputError::TextHasControl);
                }
                Ok(())
            }
        }
    }
}

fn check_modifiers(modifiers: Modifiers) -> Result<(), InputError> {
    Modifiers::from_bits(modifiers.bits())
        .map(|_| ())
        .ok_or(InputError::Modifiers)
}

/// Checks a batch of events: how many there are, and each of them.
pub fn validate_batch(events: &[Event]) -> Result<(), InputError> {
    if events.len() > MAX_BATCH {
        return Err(InputError::TooManyEvents);
    }
    events.iter().try_for_each(Event::validate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pointer(x: f32, y: f32, action: PointerAction) -> Event {
        Event::Pointer {
            frame: 0,
            x,
            y,
            modifiers: Modifiers::NONE,
            action,
        }
    }

    fn key(modifiers: Modifiers, text: Option<&str>) -> Event {
        Event::Key {
            code: 0,
            down: true,
            modifiers,
            repeat: false,
            text: text.map(str::to_owned),
        }
    }

    #[test]
    fn numbers_that_cannot_be_right_are_refused() {
        let move_to = |x, y| pointer(x, y, PointerAction::Move);
        assert_eq!(
            move_to(f32::NAN, 0.5).validate(),
            Err(InputError::NotFinite)
        );
        assert_eq!(
            move_to(0.5, f32::INFINITY).validate(),
            Err(InputError::NotFinite)
        );
        assert_eq!(move_to(-0.1, 0.5).validate(), Err(InputError::OutOfRange));
        assert_eq!(move_to(0.5, 1.1).validate(), Err(InputError::OutOfRange));
        assert_eq!(move_to(0.0, 1.0).validate(), Ok(()));
        let scroll = |dx, dy| pointer(0.5, 0.5, PointerAction::Scroll { dx, dy });
        assert_eq!(scroll(f32::NAN, 0.0).validate(), Err(InputError::NotFinite));
        assert_eq!(
            scroll(0.0, MAX_SCROLL * 2.0).validate(),
            Err(InputError::ScrollTooLarge)
        );
        assert_eq!(scroll(-MAX_SCROLL, MAX_SCROLL).validate(), Ok(()));
    }

    #[test]
    fn the_text_of_a_key_is_bounded_and_cannot_hold_control_characters() {
        assert_eq!(key(Modifiers::NONE, None).validate(), Ok(()));
        assert_eq!(key(Modifiers::NONE, Some("é\n")).validate(), Ok(()));
        let exactly = "a".repeat(MAX_KEY_TEXT_BYTES);
        assert_eq!(key(Modifiers::NONE, Some(&exactly)).validate(), Ok(()));
        let long = "a".repeat(MAX_KEY_TEXT_BYTES + 1);
        assert_eq!(
            key(Modifiers::NONE, Some(&long)).validate(),
            Err(InputError::TextTooLong)
        );
        // The limit is in bytes, not in characters.
        let wide = "é".repeat(MAX_KEY_TEXT_BYTES / 2 + 1);
        assert_eq!(
            key(Modifiers::NONE, Some(&wide)).validate(),
            Err(InputError::TextTooLong)
        );
        for control in ["a\u{1b}", "\0", "\u{7f}", "\u{85}"] {
            assert_eq!(
                key(Modifiers::NONE, Some(control)).validate(),
                Err(InputError::TextHasControl),
                "{control:?}"
            );
        }
    }

    #[test]
    fn modifiers_with_bits_that_are_not_defined_are_refused_in_both_kinds_of_event() {
        // A hostile viewer can send any byte; decoding accepts it and validation refuses it.
        let bad: Modifiers = ciborium::from_reader(&[0x18, 0x20][..]).expect("a byte");
        assert_eq!(bad.bits(), 0b10_0000);
        assert_eq!(key(bad, None).validate(), Err(InputError::Modifiers), "key");
        let on_pointer = Event::Pointer {
            frame: 0,
            x: 0.5,
            y: 0.5,
            modifiers: bad,
            action: PointerAction::Move,
        };
        assert_eq!(on_pointer.validate(), Err(InputError::Modifiers));
        assert_eq!(
            Modifiers::from_bits(0b1_1111).map(Modifiers::bits),
            Some(0b1_1111)
        );
        assert_eq!(Modifiers::from_bits(0b10_0000), None);
        assert!((Modifiers::SHIFT | Modifiers::OPTION).contains(Modifiers::OPTION));
        assert!(!Modifiers::SHIFT.contains(Modifiers::COMMAND));
    }

    #[test]
    fn a_batch_has_at_most_so_many_events() {
        let event = pointer(0.5, 0.5, PointerAction::Move);
        assert_eq!(validate_batch(&vec![event.clone(); MAX_BATCH]), Ok(()));
        assert_eq!(
            validate_batch(&vec![event.clone(); MAX_BATCH + 1]),
            Err(InputError::TooManyEvents)
        );
        // One bad event spoils the batch.
        let bad = pointer(2.0, 0.5, PointerAction::Move);
        assert_eq!(validate_batch(&[event, bad]), Err(InputError::OutOfRange));
        assert_eq!(validate_batch(&[]), Ok(()));
    }

    #[test]
    fn what_was_typed_is_not_printed() {
        let event = Event::Key {
            code: 35,
            down: true,
            modifiers: Modifiers::NONE,
            repeat: false,
            text: Some("hunter2".into()),
        };
        let shown = format!("{event:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(!shown.contains("35"), "{shown}");
        assert!(shown.contains("text_bytes: Some(7)"), "{shown}");
    }

    #[test]
    fn an_event_survives_cbor() {
        let events = vec![
            pointer(0.25, 0.75, PointerAction::Down(Button::Left)),
            pointer(0.0, 1.0, PointerAction::Scroll { dx: -3.5, dy: 12.0 }),
            key(Modifiers::SHIFT | Modifiers::COMMAND, Some("A")),
            key(Modifiers::NONE, None),
        ];
        let mut bytes = Vec::new();
        ciborium::into_writer(&events, &mut bytes).expect("encode");
        let back: Vec<Event> = ciborium::from_reader(&bytes[..]).expect("decode");
        assert_eq!(back, events);
    }
}
