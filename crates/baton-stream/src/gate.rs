// SPDX-License-Identifier: Apache-2.0

//! The rules that decide what a viewer's input does on the Mac.
//!
//! [`Gate`] looks at each event that a viewer sent, with what the Mac is doing at that moment
//! ([`Context`]), and says either what to post to the shared window ([`Post`]) or why the event
//! is dropped ([`Reason`]). The rules are those of ADR 0009:
//!
//! - **A press is dropped** unless control is on, the session runs, the Mac is unlocked, the
//!   window is on screen, the frame the event names is one the source kept and its picture went
//!   out of date no more than [`MAX_STALE`] ago, the window has not changed its size since that
//!   frame, the point is inside the window, and no other window is in front of it at that point.
//!   A press is a button going down, a pointer that moves or scrolls, or a key going down. A key
//!   names no frame and has no point, so it skips those conditions.
//! - **A key goes only to the shared window**: it needs the keyboard to be in that window, and
//!   no window of the same application in front of it that overlaps it (a menu takes keys
//!   without being the focused window).
//! - **A press that was delivered always gets its release**, whatever the other rules say, and
//!   [`Gate::release_all`] releases everything when the source stops delivering. A drag that
//!   leaves the window goes on at its edge, and positions are put on the screen relative to
//!   where the window was at the press, so that a window dragged by its title bar follows the
//!   finger and does not run ahead of it.
//!
//! A picture is out of date from the moment the window changes and not from the moment it was
//! captured. A window that does not change delivers no new frame, so its last picture is current
//! however old it is, and a tap on a still window works.
//!
//! Like the rest of the crate this is a state machine without input or output. The platform
//! tells it about frames ([`Gate::record_frame`], [`Gate::record_change`]) and about the Mac
//! (every call to [`Gate::handle`] carries a [`Context`]), and posts what it answers.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::input::{Button, Event, MAX_EVENTS_PER_SECOND, Modifiers, PointerAction};

/// How long a picture may have been out of date when an event names it.
pub const MAX_STALE: Duration = Duration::from_millis(500);

/// How long the geometry of a frame is kept after its picture went out of date.
pub const GEOMETRY_KEPT: Duration = Duration::from_secs(1);

/// Most keys held down at once. A person has ten fingers.
pub const MAX_HELD_KEYS: usize = 32;

/// Most frames whose geometry is kept, whatever the frame rate.
const MAX_FRAMES_KEPT: usize = 512;

/// Most changes of the picture that are remembered.
const MAX_CHANGES_KEPT: usize = 1024;

/// How far two window sizes may differ, in points, and still be the same size.
const SAME_SIZE: f64 = 0.5;

/// A point, in points on the screen or in a window, or in pixels in a picture.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    /// Distance from the left edge.
    pub x: f64,
    /// Distance from the top edge.
    pub y: f64,
}

/// A size, in the same units as [`Point`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

impl Size {
    fn is_sound(&self) -> bool {
        self.width.is_finite() && self.height.is_finite() && self.width > 0.0 && self.height > 0.0
    }

    fn same_as(&self, other: &Self) -> bool {
        (self.width - other.width).abs() <= SAME_SIZE
            && (self.height - other.height).abs() <= SAME_SIZE
    }
}

/// A rectangle: its top left corner and its size.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// The top left corner.
    pub origin: Point,
    /// The size.
    pub size: Size,
}

impl Rect {
    /// A rectangle from its corner and its size.
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            origin: Point { x, y },
            size: Size { width, height },
        }
    }

    fn right(&self) -> f64 {
        self.origin.x + self.size.width
    }

    fn bottom(&self) -> f64 {
        self.origin.y + self.size.height
    }

    /// Whether the rectangle has a finite position and a finite, positive size.
    fn is_sound(&self) -> bool {
        self.origin.x.is_finite() && self.origin.y.is_finite() && self.size.is_sound()
    }

    /// Whether the point is inside. The right and bottom edges are outside, as on a screen.
    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.origin.x
            && point.x < self.right()
            && point.y >= self.origin.y
            && point.y < self.bottom()
    }

    /// Whether the two rectangles have area in common. Rectangles that only touch do not.
    pub fn overlaps(&self, other: &Self) -> bool {
        self.origin.x < other.right()
            && other.origin.x < self.right()
            && self.origin.y < other.bottom()
            && other.origin.y < self.bottom()
    }

    /// The point nearest to `point` that is inside, one point away from the far edges, because
    /// a point on the right or bottom edge belongs to whatever lies next to the rectangle. A
    /// rectangle that cannot be used leaves the point as it is.
    fn nearest_inside(&self, point: Point) -> Point {
        if !self.is_sound() {
            return point;
        }
        Point {
            x: point
                .x
                .clamp(self.origin.x, self.origin.x + inset(self.size.width)),
            y: point
                .y
                .clamp(self.origin.y, self.origin.y + inset(self.size.height)),
        }
    }
}

/// The largest position inside a span: one point short of its far end, or its start if the
/// span is shorter than a point.
fn inset(span: f64) -> f64 {
    (span - 1.0).max(0.0)
}

/// How the shared window was laid out in a frame, which is what turns a position in the picture
/// into a position in the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    /// The size of the window on the screen, in points.
    pub window: Size,
    /// The size of the picture, in pixels.
    pub picture: Size,
    /// Where the content of the window is in the picture, in pixels. The rest is margin.
    pub content: Rect,
}

impl Geometry {
    fn is_sound(&self) -> bool {
        self.window.is_sound() && self.picture.is_sound() && self.content.is_sound()
    }

    /// Where in the window a position in the picture lies, in points from the window's top left
    /// corner, and whether it lies on the content. A position in the margin is moved to the
    /// nearest point of the content, and the result is at most one point short of the far edges
    /// of the window.
    fn locate(&self, across: f64, down: f64) -> (Point, bool) {
        let pixel_x = across * self.picture.width;
        let pixel_y = down * self.picture.height;
        let inside = pixel_x >= self.content.origin.x
            && pixel_x <= self.content.right()
            && pixel_y >= self.content.origin.y
            && pixel_y <= self.content.bottom();
        let on_x = pixel_x.clamp(self.content.origin.x, self.content.right());
        let on_y = pixel_y.clamp(self.content.origin.y, self.content.bottom());
        let x = (on_x - self.content.origin.x) / self.content.size.width * self.window.width;
        let y = (on_y - self.content.origin.y) / self.content.size.height * self.window.height;
        let point = Point {
            x: x.clamp(0.0, inset(self.window.width)),
            y: y.clamp(0.0, inset(self.window.height)),
        };
        (point, inside)
    }
}

/// Why an event was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Reason {
    /// The event has a number or a length that is not allowed. A viewer that sends this is
    /// already in violation (the batch is checked first), so this is for the gate on its own.
    Malformed,
    /// Control is off.
    ControlOff,
    /// The session is paused.
    Paused,
    /// The Mac is locked, or its display is off.
    Locked,
    /// The shared window is not on screen.
    NotOnScreen,
    /// The viewer sent more events in a second than it may.
    TooFast,
    /// The frame that the event names is not one whose geometry is kept.
    FrameUnknown,
    /// The picture that the event names went out of date too long ago for the event to mean
    /// what it did.
    PictureOutOfDate,
    /// The window has changed its size since that frame.
    WindowResized,
    /// The point is outside the window.
    OutsideWindow,
    /// Another window is in front of the window at that point.
    Covered,
    /// The shared window does not have the keyboard.
    NoKeyboard,
    /// A window of the same application lies in front of the shared window and overlaps it, or
    /// is an open menu wherever it is, so a key might reach it and not the shared window.
    AppWindowInFront,
    /// More keys are held down than a person has fingers.
    TooManyHeld,
    /// A release of a button or a key that this session did not press.
    NotPressed,
}

/// Whether the shared window has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyboard {
    /// The application is in front and the shared window is its focused window.
    Shared,
    /// Another window has the keyboard, or the application is not in front.
    Elsewhere,
    /// The source could not tell which window has it. That counts as not the shared one.
    Unknown,
}

/// A window that is in front of the shared window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Front {
    /// Where it is, in points on the screen.
    pub rect: Rect,
    /// Whether it belongs to the same application as the shared window.
    pub same_application: bool,
    /// Whether it is on the layer of an open menu, which R6 records. A menu takes the keys
    /// wherever it opens, under the menu bar for example, so for a key it counts whether or not
    /// it overlaps the shared window. For a press it is a window like another.
    pub menu: bool,
}

/// What the platform knows when an event arrives.
// These are four separate facts about the session, each of which has a reason of its own, and a
// state that combines them would only hide which one dropped an event.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    /// The time by the source's clock, which is also the clock of the capture times that
    /// [`Gate::record_frame`] and [`Gate::record_change`] are given.
    pub now: Duration,
    /// Whether the person at the Mac has control turned on for this session.
    pub control: bool,
    /// Whether the session is paused.
    pub paused: bool,
    /// Whether the Mac is locked or its display is off.
    pub locked: bool,
    /// Whether the shared window is on screen.
    pub on_screen: bool,
    /// Where the shared window is now, in points on the screen.
    pub window: Rect,
    /// The windows that are in front of the shared window and draw something. The platform
    /// leaves out those that are fully transparent, and, for the layers that R6 shows do not
    /// take keys or cover anything, those too.
    pub in_front: &'a [Front],
    /// Whether the shared window has the keyboard.
    pub keyboard: Keyboard,
}

/// What to post to the shared window.
#[derive(Clone, PartialEq)]
pub enum Post {
    /// A pointer event, at a point on the screen.
    Pointer {
        /// Where, in points on the screen.
        at: Point,
        /// The modifiers that are down.
        modifiers: Modifiers,
        /// What the pointer did.
        action: PointerAction,
    },
    /// A key event.
    Key {
        /// The virtual key code.
        code: u16,
        /// Whether the key went down.
        down: bool,
        /// The modifiers that are down after this event.
        modifiers: Modifiers,
        /// Whether the system repeats a key that is held.
        repeat: bool,
        /// The text that the key produced.
        text: Option<String>,
    },
}

// What was typed stays out of every log, as for the event it comes from.
impl fmt::Debug for Post {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pointer {
                at,
                modifiers,
                action,
            } => f
                .debug_struct("Pointer")
                .field("at", at)
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

/// What to do with an event.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Post this to the process of the shared window.
    Post(Post),
    /// Do nothing, and the reason.
    Drop(Reason),
}

#[derive(Debug, Clone, Copy)]
struct Recorded {
    number: u64,
    /// When the picture of the frame was captured, by the source's clock.
    picture: Duration,
    geometry: Geometry,
}

/// What the gate needs of a key event.
struct KeyEvent<'a> {
    code: u16,
    modifiers: Modifiers,
    repeat: bool,
    text: Option<&'a str>,
}

impl KeyEvent<'_> {
    fn post(&self, down: bool) -> Post {
        Post::Key {
            code: self.code,
            down,
            modifiers: self.modifiers,
            repeat: self.repeat,
            text: self.text.map(str::to_owned),
        }
    }
}

/// The rules for events from a viewer, with what they need to remember: the geometry of recent
/// frames, when the picture changed, and what has been pressed.
#[derive(Debug, Default)]
pub struct Gate {
    /// The frames that were sent, oldest first.
    frames: VecDeque<Recorded>,
    /// The capture times of the pictures that showed a change, oldest first.
    changes: VecDeque<Duration>,
    buttons: BTreeSet<Button>,
    keys: BTreeSet<u16>,
    /// Where the window was when the first button went down, while a button is down.
    anchor: Option<Point>,
    /// Where the pointer was last put, in points from the top left corner of the window.
    last: Option<Point>,
    /// The times of the events of the last second that count against the rate.
    recent: VecDeque<Duration>,
}

impl Gate {
    /// A gate with nothing pressed and no frames.
    pub fn new() -> Self {
        Self::default()
    }

    /// The window showed a change at `picture`, the capture time of the new picture. Call it for
    /// every picture that shows a change, whether it is sent or dropped before the queue: the
    /// first change after a frame is when the picture of that frame went out of date.
    pub fn record_change(&mut self, picture: Duration) {
        if let Err(at) = self.changes.binary_search(&picture) {
            self.changes.insert(at, picture);
        }
        while self.changes.len() > MAX_CHANGES_KEPT {
            self.changes.pop_front();
        }
    }

    /// A frame was sent. `picture` is when the picture in it was captured, and `geometry` how
    /// the window was laid out in it. A frame that encodes the same picture again gets the
    /// capture time of that picture, so it belongs to it and does not replace it. A geometry that
    /// cannot be used is not remembered, and events that name its frame are dropped.
    ///
    /// A frame is also a change at its capture time if none is known, so a platform that only
    /// calls this still gets the picture that it sent right.
    pub fn record_frame(&mut self, number: u64, picture: Duration, geometry: Geometry) {
        self.record_change(picture);
        if !geometry.is_sound() {
            return;
        }
        self.frames.push_back(Recorded {
            number,
            picture,
            geometry,
        });
        while self.frames.len() > MAX_FRAMES_KEPT {
            self.frames.pop_front();
        }
    }

    /// Whether anything is held down.
    pub fn is_pressing(&self) -> bool {
        !self.buttons.is_empty() || !self.keys.is_empty()
    }

    /// Decides what to do with an event.
    pub fn handle(&mut self, context: &Context<'_>, event: &Event) -> Decision {
        if event.validate().is_err() {
            return Decision::Drop(Reason::Malformed);
        }
        self.forget_old(context.now);
        match event {
            Event::Pointer {
                frame,
                x,
                y,
                modifiers,
                action,
            } => self.pointer(
                context,
                *frame,
                (f64::from(*x), f64::from(*y)),
                *modifiers,
                *action,
            ),
            Event::Key {
                code,
                down,
                modifiers,
                repeat,
                text,
            } => {
                let key = KeyEvent {
                    code: *code,
                    modifiers: *modifiers,
                    repeat: *repeat,
                    text: text.as_deref(),
                };
                if *down {
                    self.key_down(context, &key)
                } else {
                    self.key_up(&key)
                }
            }
        }
    }

    /// What to post when the source stops delivering (control turned off, a pause, a lock, the
    /// end of the session, a viewer that went quiet): a release for everything that was pressed.
    /// `window` is where the shared window is now, if it still is anywhere.
    pub fn release_all(&mut self, window: Rect) -> Vec<Post> {
        let mut posts = Vec::new();
        let at = self.release_point(&window, None);
        for button in std::mem::take(&mut self.buttons) {
            posts.push(Post::Pointer {
                at,
                modifiers: Modifiers::NONE,
                action: PointerAction::Up(button),
            });
        }
        for code in std::mem::take(&mut self.keys) {
            posts.push(Post::Key {
                code,
                down: false,
                modifiers: Modifiers::NONE,
                repeat: false,
                text: None,
            });
        }
        self.anchor = None;
        posts
    }

    /// When the picture captured at `picture` went out of date: the first change after it.
    fn out_of_date_at(&self, picture: Duration) -> Option<Duration> {
        let after = self.changes.partition_point(|change| *change <= picture);
        self.changes.get(after).copied()
    }

    fn forget_old(&mut self, now: Duration) {
        while self.frames.front().is_some_and(|front| {
            self.out_of_date_at(front.picture)
                .is_some_and(|at| now.saturating_sub(at) > GEOMETRY_KEPT)
        }) {
            self.frames.pop_front();
        }
        // A change that is not after the oldest picture kept cannot make anything out of date.
        if let Some(oldest) = self.frames.front().map(|front| front.picture) {
            while self.changes.front().is_some_and(|change| *change <= oldest) {
                self.changes.pop_front();
            }
        }
        while self
            .recent
            .front()
            .is_some_and(|at| now.saturating_sub(*at) >= Duration::from_secs(1))
        {
            self.recent.pop_front();
        }
    }

    /// The checks that every press passes first, and the count against the rate.
    fn check_session(&mut self, context: &Context<'_>) -> Result<(), Reason> {
        if !context.control {
            return Err(Reason::ControlOff);
        }
        if context.paused {
            return Err(Reason::Paused);
        }
        if context.locked {
            return Err(Reason::Locked);
        }
        if !context.on_screen || !context.window.is_sound() {
            return Err(Reason::NotOnScreen);
        }
        if self.recent.len() >= MAX_EVENTS_PER_SECOND {
            return Err(Reason::TooFast);
        }
        self.recent.push_back(context.now);
        Ok(())
    }

    /// The geometry of the frame that an event names, if it is known and its picture has not
    /// been out of date for too long.
    fn check_picture(&self, now: Duration, frame: u64) -> Result<Geometry, Reason> {
        let recorded = self.recorded(frame).ok_or(Reason::FrameUnknown)?;
        let stale = self
            .out_of_date_at(recorded.picture)
            .is_some_and(|at| now.saturating_sub(at) > MAX_STALE);
        if stale {
            return Err(Reason::PictureOutOfDate);
        }
        Ok(recorded.geometry)
    }

    fn recorded(&self, frame: u64) -> Option<&Recorded> {
        self.frames.iter().rev().find(|f| f.number == frame)
    }

    fn pointer(
        &mut self,
        context: &Context<'_>,
        frame: u64,
        (across, down): (f64, f64),
        modifiers: Modifiers,
        action: PointerAction,
    ) -> Decision {
        if let PointerAction::Up(button) = action {
            return self.pointer_up(context, frame, (across, down), modifiers, button);
        }
        if let Err(reason) = self.check_session(context) {
            return Decision::Drop(reason);
        }
        let geometry = match self.check_picture(context.now, frame) {
            Ok(geometry) => geometry,
            Err(reason) => return Decision::Drop(reason),
        };
        // The same position in the picture would be another spot in a window of another size. A
        // window that only moved is no reason: the position is added to where it is now.
        if !geometry.window.same_as(&context.window.size) {
            return Decision::Drop(Reason::WindowResized);
        }
        let (relative, inside) = geometry.locate(across, down);
        // A drag that leaves the window goes on at its edge. Anything else that is outside was
        // not meant for this window.
        if !inside && self.buttons.is_empty() {
            return Decision::Drop(Reason::OutsideWindow);
        }
        let origin = self.anchor.unwrap_or(context.window.origin);
        let at = offset(origin, relative);
        if covered(context.in_front, at) {
            return Decision::Drop(Reason::Covered);
        }
        if let PointerAction::Down(button) = action {
            if self.buttons.is_empty() {
                self.anchor = Some(context.window.origin);
            }
            self.buttons.insert(button);
        }
        self.last = Some(relative);
        Decision::Post(Post::Pointer {
            at,
            modifiers,
            action,
        })
    }

    fn pointer_up(
        &mut self,
        context: &Context<'_>,
        frame: u64,
        (across, down): (f64, f64),
        modifiers: Modifiers,
        button: Button,
    ) -> Decision {
        if !self.buttons.remove(&button) {
            return Decision::Drop(Reason::NotPressed);
        }
        // Where the viewer lifted its finger, if that still means something.
        let named = self
            .recorded(frame)
            .filter(|f| f.geometry.window.same_as(&context.window.size))
            .map(|f| f.geometry.locate(across, down).0);
        let at = self.release_point(&context.window, Some((named, context.in_front)));
        if self.buttons.is_empty() {
            self.anchor = None;
        }
        if let Some(named) = named {
            self.last = Some(named);
        }
        Decision::Post(Post::Pointer {
            at,
            modifiers,
            action: PointerAction::Up(button),
        })
    }

    /// Where a release is put, as a point on the screen inside `window` as it is now.
    ///
    /// The release goes where the viewer lifted its finger if nothing is in front of the window
    /// there, and where the pointer was last put otherwise, so that a window that appeared during
    /// the press does not receive the end of it.
    fn release_point(&self, window: &Rect, named: Option<(Option<Point>, &[Front])>) -> Point {
        let origin = self.anchor.unwrap_or(window.origin);
        let last = window.nearest_inside(offset(origin, self.last.unwrap_or_default()));
        let Some((Some(relative), in_front)) = named else {
            return last;
        };
        let wanted = window.nearest_inside(offset(origin, relative));
        if covered(in_front, wanted) {
            last
        } else {
            wanted
        }
    }

    fn key_down(&mut self, context: &Context<'_>, key: &KeyEvent<'_>) -> Decision {
        if let Err(reason) = self.check_session(context) {
            return Decision::Drop(reason);
        }
        if context.keyboard != Keyboard::Shared {
            return Decision::Drop(Reason::NoKeyboard);
        }
        // A menu takes the keys without being the focused window, so a window of the same
        // application in front of the shared one might be where the key goes. A menu does
        // wherever it is on the screen, and any other window does if it overlaps.
        if context.in_front.iter().any(|front| {
            front.same_application && (front.menu || front.rect.overlaps(&context.window))
        }) {
            return Decision::Drop(Reason::AppWindowInFront);
        }
        if !self.keys.contains(&key.code) && self.keys.len() >= MAX_HELD_KEYS {
            return Decision::Drop(Reason::TooManyHeld);
        }
        self.keys.insert(key.code);
        Decision::Post(key.post(true))
    }

    fn key_up(&mut self, key: &KeyEvent<'_>) -> Decision {
        if self.keys.remove(&key.code) {
            Decision::Post(key.post(false))
        } else {
            Decision::Drop(Reason::NotPressed)
        }
    }
}

/// The point on the screen for a position in points from the top left corner of a window that
/// was at `origin`.
fn offset(origin: Point, relative: Point) -> Point {
    Point {
        x: origin.x + relative.x,
        y: origin.y + relative.y,
    }
}

/// Whether a window in front covers the point.
fn covered(in_front: &[Front], at: Point) -> bool {
    in_front.iter().any(|front| front.rect.contains(at))
}

/// One thing to tell the viewer about events that were dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    /// The rule that dropped the most events since the last notice.
    pub reason: Reason,
    /// How many events it dropped.
    pub count: u32,
}

/// Collects the reasons for dropped events and gives at most one notice in a second, so that a
/// tap that did nothing has a reason that the person can read, and a flood costs one message.
/// A notice holds no title and no text that was typed.
#[derive(Debug, Default)]
pub struct Notices {
    counts: BTreeMap<Reason, u32>,
    last: Option<Duration>,
}

impl Notices {
    /// No notices.
    pub fn new() -> Self {
        Self::default()
    }

    /// An event was dropped for this reason.
    pub fn dropped(&mut self, reason: Reason) {
        let count = self.counts.entry(reason).or_insert(0);
        *count = count.saturating_add(1);
    }

    /// The notice to send now, if there is one and the last was a second ago or more.
    pub fn poll(&mut self, now: Duration) -> Option<Notice> {
        if self
            .last
            .is_some_and(|last| now.saturating_sub(last) < Duration::from_secs(1))
        {
            return None;
        }
        // The reason with the most events, and the smallest of those that tie.
        let (reason, count) = self.counts.iter().fold(
            None,
            |best: Option<(Reason, u32)>, (reason, count)| match best {
                Some((_, most)) if most >= *count => best,
                _ => Some((*reason, *count)),
            },
        )?;
        self.counts.clear();
        self.last = Some(now);
        Some(Notice { reason, count })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn window() -> Rect {
        Rect::new(100.0, 100.0, 800.0, 600.0)
    }

    /// A picture twice the size of the window, with the window filling it.
    fn geometry() -> Geometry {
        Geometry {
            window: Size {
                width: 800.0,
                height: 600.0,
            },
            picture: Size {
                width: 1600.0,
                height: 1200.0,
            },
            content: Rect::new(0.0, 0.0, 1600.0, 1200.0),
        }
    }

    /// The window shown in the middle of a wider picture, with margin on both sides.
    fn geometry_with_margin() -> Geometry {
        let mut geometry = geometry();
        geometry.picture.width = 2000.0;
        geometry.content = Rect::new(200.0, 0.0, 1600.0, 1200.0);
        geometry
    }

    /// A change to the context of a test: what the Mac is doing differently.
    type Change = fn(&mut Context<'_>);

    fn context(now: Duration, in_front: &[Front]) -> Context<'_> {
        Context {
            now,
            control: true,
            paused: false,
            locked: false,
            on_screen: true,
            window: window(),
            in_front,
            keyboard: Keyboard::Shared,
        }
    }

    /// A gate that has sent frame 7, whose picture was captured at time zero.
    fn gate_with_frame() -> Gate {
        let mut gate = Gate::new();
        gate.record_frame(7, Duration::ZERO, geometry());
        gate
    }

    fn pointer(frame: u64, x: f32, y: f32, action: PointerAction) -> Event {
        pointer_with(frame, x, y, Modifiers::NONE, action)
    }

    fn pointer_with(
        frame: u64,
        x: f32,
        y: f32,
        modifiers: Modifiers,
        action: PointerAction,
    ) -> Event {
        Event::Pointer {
            frame,
            x,
            y,
            modifiers,
            action,
        }
    }

    fn key(code: u16, down: bool) -> Event {
        Event::Key {
            code,
            down,
            modifiers: Modifiers::NONE,
            repeat: false,
            text: None,
        }
    }

    fn posted(decision: Decision) -> Post {
        match decision {
            Decision::Post(post) => post,
            Decision::Drop(reason) => panic!("dropped: {reason:?}"),
        }
    }

    fn dropped(decision: Decision) -> Reason {
        match decision {
            Decision::Drop(reason) => reason,
            Decision::Post(post) => panic!("posted: {post:?}"),
        }
    }

    fn other_app(rect: Rect) -> Front {
        Front {
            rect,
            same_application: false,
            menu: false,
        }
    }

    fn same_app(rect: Rect) -> Front {
        Front {
            rect,
            same_application: true,
            menu: false,
        }
    }

    fn menu_of_app(rect: Rect) -> Front {
        Front {
            menu: true,
            ..same_app(rect)
        }
    }

    fn at(x: f64, y: f64, action: PointerAction) -> Post {
        Post::Pointer {
            at: Point { x, y },
            modifiers: Modifiers::NONE,
            action,
        }
    }

    #[test]
    fn a_click_inside_the_window_is_posted_at_the_spot_on_the_screen() {
        let mut gate = gate_with_frame();
        let context = context(10 * MS, &[]);
        let event = pointer(7, 0.5, 0.25, PointerAction::Down(Button::Left));
        // Half across the picture and a quarter down: 400 and 150 points into the window.
        assert_eq!(
            posted(gate.handle(&context, &event)),
            at(500.0, 250.0, PointerAction::Down(Button::Left))
        );
    }

    #[test]
    fn modifiers_travel_with_a_pointer_event() {
        let mut gate = gate_with_frame();
        let context = context(10 * MS, &[]);
        let modifiers = Modifiers::SHIFT | Modifiers::COMMAND;
        let event = pointer_with(7, 0.5, 0.5, modifiers, PointerAction::Down(Button::Left));
        assert_eq!(
            posted(gate.handle(&context, &event)),
            Post::Pointer {
                at: Point { x: 500.0, y: 400.0 },
                modifiers,
                action: PointerAction::Down(Button::Left)
            }
        );
    }

    #[test]
    fn the_margin_of_the_picture_is_not_the_window() {
        let mut gate = Gate::new();
        gate.record_frame(7, Duration::ZERO, geometry_with_margin());
        let context = context(10 * MS, &[]);
        let margin = pointer(7, 0.05, 0.5, PointerAction::Down(Button::Left));
        assert_eq!(
            dropped(gate.handle(&context, &margin)),
            Reason::OutsideWindow
        );
        // The centre of the picture is the centre of the window.
        let centre = pointer(7, 0.5, 0.5, PointerAction::Move);
        assert_eq!(
            posted(gate.handle(&context, &centre)),
            at(500.0, 400.0, PointerAction::Move)
        );
    }

    #[test]
    fn each_condition_of_the_session_drops_a_press() {
        let cases: [(Change, Reason); 4] = [
            (|c| c.control = false, Reason::ControlOff),
            (|c| c.paused = true, Reason::Paused),
            (|c| c.locked = true, Reason::Locked),
            (|c| c.on_screen = false, Reason::NotOnScreen),
        ];
        for (change, reason) in cases {
            for event in [
                pointer(7, 0.5, 0.5, PointerAction::Down(Button::Left)),
                pointer(7, 0.5, 0.5, PointerAction::Move),
                pointer(7, 0.5, 0.5, PointerAction::Scroll { dx: 0.0, dy: 3.0 }),
                key(4, true),
            ] {
                let mut gate = gate_with_frame();
                let mut context = context(10 * MS, &[]);
                change(&mut context);
                assert_eq!(dropped(gate.handle(&context, &event)), reason, "{event:?}");
                assert!(!gate.is_pressing());
            }
        }
    }

    #[test]
    fn a_picture_may_have_been_out_of_date_for_half_a_second() {
        let mut gate = gate_with_frame();
        // The window changed at 100 ms: from then on the picture of frame 7 is out of date.
        gate.record_change(100 * MS);
        let event = pointer(7, 0.5, 0.5, PointerAction::Move);
        let at_limit = context(600 * MS, &[]);
        let just_past = context(601 * MS, &[]);
        assert!(matches!(gate.handle(&at_limit, &event), Decision::Post(_)));
        assert_eq!(
            dropped(gate.handle(&just_past, &event)),
            Reason::PictureOutOfDate
        );
        // A key names no frame, so what the picture is does not matter to it.
        assert!(matches!(
            gate.handle(&just_past, &key(4, true)),
            Decision::Post(_)
        ));
    }

    #[test]
    fn a_window_that_has_not_changed_has_a_current_picture_however_old() {
        let mut gate = gate_with_frame();
        // Nothing changed for ten minutes, so no frame was sent, and a tap still means what it
        // did on the picture that the person sees.
        let later = context(600 * 1000 * MS, &[]);
        let tap = pointer(7, 0.5, 0.5, PointerAction::Down(Button::Left));
        assert_eq!(
            posted(gate.handle(&later, &tap)),
            at(500.0, 400.0, PointerAction::Down(Button::Left))
        );
        // Until the window changes: from then on the clock runs.
        gate.record_change(600 * 1000 * MS + 50 * MS);
        gate.handle(
            &later,
            &pointer(7, 0.5, 0.5, PointerAction::Up(Button::Left)),
        );
        let soon = context(600 * 1000 * MS + 500 * MS, &[]);
        let after = context(600 * 1000 * MS + 551 * MS, &[]);
        let event = pointer(7, 0.5, 0.5, PointerAction::Move);
        assert!(matches!(gate.handle(&soon, &event), Decision::Post(_)));
        assert_eq!(
            dropped(gate.handle(&after, &event)),
            Reason::PictureOutOfDate
        );
    }

    #[test]
    fn a_picture_that_was_dropped_before_the_queue_still_makes_the_old_one_out_of_date() {
        let mut gate = gate_with_frame();
        // The next picture was captured and never sent: only the change is known.
        gate.record_change(100 * MS);
        let event = pointer(7, 0.5, 0.5, PointerAction::Move);
        assert!(matches!(
            gate.handle(&context(600 * MS, &[]), &event),
            Decision::Post(_)
        ));
        assert_eq!(
            dropped(gate.handle(&context(601 * MS, &[]), &event)),
            Reason::PictureOutOfDate
        );
    }

    #[test]
    fn a_frame_that_encodes_the_same_picture_again_does_not_replace_it() {
        let mut gate = Gate::new();
        gate.record_frame(7, Duration::ZERO, geometry());
        // Frame 8 is frame 7 encoded again (a recovery frame on a still screen): same picture,
        // same capture time.
        gate.record_frame(8, Duration::ZERO, geometry());
        gate.record_change(100 * MS);
        for frame in [7, 8] {
            let event = pointer(frame, 0.5, 0.5, PointerAction::Move);
            assert!(
                matches!(
                    gate.handle(&context(550 * MS, &[]), &event),
                    Decision::Post(_)
                ),
                "frame {frame}"
            );
        }
        for frame in [7, 8] {
            let event = pointer(frame, 0.5, 0.5, PointerAction::Move);
            assert_eq!(
                dropped(gate.handle(&context(601 * MS, &[]), &event)),
                Reason::PictureOutOfDate,
                "frame {frame}"
            );
        }
    }

    #[test]
    fn the_geometry_of_a_frame_is_kept_for_a_second_after_its_picture_went_out_of_date() {
        let mut gate = gate_with_frame();
        gate.record_change(100 * MS);
        let event = pointer(7, 0.5, 0.5, PointerAction::Move);
        assert_eq!(
            dropped(gate.handle(&context(1100 * MS, &[]), &event)),
            Reason::PictureOutOfDate
        );
        assert_eq!(
            dropped(gate.handle(&context(1101 * MS, &[]), &event)),
            Reason::FrameUnknown
        );
        assert!(gate.frames.is_empty());
        // What nothing needs any more is not kept either.
        assert!(gate.changes.is_empty() || gate.changes.len() == 1);
    }

    #[test]
    fn a_frame_that_was_never_sent_is_unknown() {
        let mut gate = gate_with_frame();
        let event = pointer(8, 0.5, 0.5, PointerAction::Down(Button::Left));
        assert_eq!(
            dropped(gate.handle(&context(10 * MS, &[]), &event)),
            Reason::FrameUnknown
        );
    }

    #[test]
    fn a_window_that_only_moved_is_no_reason_and_one_that_changed_its_size_is() {
        let event = pointer(7, 0.5, 0.25, PointerAction::Move);
        // The position is added to where the window is now.
        let mut gate = gate_with_frame();
        let mut moved = context(10 * MS, &[]);
        moved.window = Rect::new(110.0, 100.0, 800.0, 600.0);
        assert_eq!(
            posted(gate.handle(&moved, &event)),
            at(510.0, 250.0, PointerAction::Move)
        );
        for resized in [
            Rect::new(100.0, 100.0, 801.0, 600.0),
            Rect::new(100.0, 100.0, 800.0, 590.0),
            Rect::new(0.0, 0.0, 400.0, 300.0),
        ] {
            let mut gate = gate_with_frame();
            let mut context = context(10 * MS, &[]);
            context.window = resized;
            assert_eq!(
                dropped(gate.handle(&context, &event)),
                Reason::WindowResized,
                "{resized:?}"
            );
        }
        // Half a point is no change of size.
        let mut gate = gate_with_frame();
        let mut context = context(10 * MS, &[]);
        context.window = Rect::new(100.0, 100.0, 800.4, 599.6);
        assert!(matches!(gate.handle(&context, &event), Decision::Post(_)));
    }

    #[test]
    fn a_point_on_the_edge_is_one_point_inside_the_window() {
        let mut gate = gate_with_frame();
        let context = context(10 * MS, &[]);
        let corner = pointer(7, 0.0, 0.0, PointerAction::Move);
        assert_eq!(
            posted(gate.handle(&context, &corner)),
            at(100.0, 100.0, PointerAction::Move)
        );
        // The far edge belongs to whatever lies next to the window.
        let far = pointer(7, 1.0, 1.0, PointerAction::Move);
        assert_eq!(
            posted(gate.handle(&context, &far)),
            at(899.0, 699.0, PointerAction::Move)
        );
    }

    #[test]
    fn a_point_with_another_window_in_front_is_dropped() {
        let sheet = [other_app(Rect::new(300.0, 150.0, 400.0, 200.0))];
        let mut gate = gate_with_frame();
        let context = context(10 * MS, &sheet);
        // (500, 250) is under the sheet; (200, 250) is not.
        let under = pointer(7, 0.5, 0.25, PointerAction::Down(Button::Left));
        let beside = pointer(7, 0.125, 0.25, PointerAction::Down(Button::Left));
        assert_eq!(dropped(gate.handle(&context, &under)), Reason::Covered);
        assert!(!gate.is_pressing());
        assert!(matches!(gate.handle(&context, &beside), Decision::Post(_)));
        assert!(gate.is_pressing());
    }

    #[test]
    fn a_key_goes_only_to_the_shared_window() {
        for keyboard in [Keyboard::Elsewhere, Keyboard::Unknown] {
            let mut gate = gate_with_frame();
            let mut context = context(10 * MS, &[]);
            context.keyboard = keyboard;
            assert_eq!(
                dropped(gate.handle(&context, &key(4, true))),
                Reason::NoKeyboard
            );
            assert!(!gate.is_pressing());
        }
        let mut gate = gate_with_frame();
        let context = context(10 * MS, &[]);
        assert!(matches!(
            gate.handle(&context, &key(4, true)),
            Decision::Post(Post::Key { down: true, .. })
        ));
    }

    #[test]
    fn a_key_is_dropped_while_a_window_of_the_same_application_lies_over_the_shared_one() {
        let key_down = key(4, true);
        // A menu of the same application that reaches into the window.
        let menu = [same_app(Rect::new(500.0, 50.0, 200.0, 150.0))];
        let mut gate = gate_with_frame();
        assert_eq!(
            dropped(gate.handle(&context(10 * MS, &menu), &key_down)),
            Reason::AppWindowInFront
        );
        assert!(!gate.is_pressing());
        // The tap that closes the menu is not dropped by the rule for keys: a point outside the
        // menu is free.
        let outside = pointer(7, 0.05, 0.05, PointerAction::Down(Button::Left));
        assert!(matches!(
            gate.handle(&context(20 * MS, &menu), &outside),
            Decision::Post(_)
        ));
        // Not the same application, not overlapping, or only touching an edge: no reason.
        for front in [
            other_app(Rect::new(500.0, 50.0, 200.0, 150.0)),
            same_app(Rect::new(1000.0, 100.0, 100.0, 100.0)),
            same_app(Rect::new(900.0, 100.0, 100.0, 100.0)),
            same_app(Rect::new(0.0, 0.0, 100.0, 100.0)),
            same_app(Rect::new(f64::NAN, 0.0, 100.0, 100.0)),
        ] {
            let mut gate = gate_with_frame();
            let fronts = [front];
            assert!(
                matches!(
                    gate.handle(&context(10 * MS, &fronts), &key_down),
                    Decision::Post(_)
                ),
                "{front:?}"
            );
        }
    }

    #[test]
    fn a_key_is_dropped_while_a_menu_of_the_application_is_open_wherever_it_is() {
        let key_down = key(4, true);
        // The Help menu opens under the menu bar, far from the shared window, and the
        // application handles the shortcut that opened it. It takes the keys all the same.
        let help = [menu_of_app(Rect::new(1000.0, 0.0, 300.0, 400.0))];
        let mut gate = gate_with_frame();
        assert_eq!(
            dropped(gate.handle(&context(10 * MS, &help), &key_down)),
            Reason::AppWindowInFront
        );
        // A tap on the shared window is not affected, and closes the menu.
        let tap = pointer(7, 0.5, 0.5, PointerAction::Down(Button::Left));
        assert!(matches!(
            gate.handle(&context(20 * MS, &help), &tap),
            Decision::Post(_)
        ));
        // A menu of another application, or a window that is not a menu and does not overlap, is
        // no reason.
        for front in [
            other_app(Rect::new(1000.0, 0.0, 300.0, 400.0)),
            Front {
                menu: true,
                ..other_app(Rect::new(1000.0, 0.0, 300.0, 400.0))
            },
            same_app(Rect::new(1000.0, 0.0, 300.0, 400.0)),
        ] {
            let mut gate = gate_with_frame();
            let fronts = [front];
            assert!(
                matches!(
                    gate.handle(&context(10 * MS, &fronts), &key_down),
                    Decision::Post(_)
                ),
                "{front:?}"
            );
        }
    }

    #[test]
    fn a_key_names_no_frame_and_has_no_point() {
        // No frame was ever sent, and a window of another application covers the middle.
        let mut gate = Gate::new();
        let fronts = [other_app(Rect::new(0.0, 0.0, 2000.0, 2000.0))];
        assert!(matches!(
            gate.handle(&context(10 * MS, &fronts), &key(4, true)),
            Decision::Post(_)
        ));
    }

    #[test]
    fn a_key_carries_what_the_viewer_sent() {
        let mut gate = gate_with_frame();
        let context = context(10 * MS, &[]);
        let event = Event::Key {
            code: 0,
            down: true,
            modifiers: Modifiers::SHIFT | Modifiers::COMMAND,
            repeat: true,
            text: Some("A".into()),
        };
        assert_eq!(
            posted(gate.handle(&context, &event)),
            Post::Key {
                code: 0,
                down: true,
                modifiers: Modifiers::SHIFT | Modifiers::COMMAND,
                repeat: true,
                text: Some("A".into())
            }
        );
    }

    #[test]
    fn a_press_that_was_delivered_always_gets_its_release() {
        // Whatever happens to the window or the session between the press and the release.
        let sheet = [other_app(Rect::new(0.0, 0.0, 2000.0, 2000.0))];
        let cases: [(&str, Change); 6] = [
            ("control off", |c| c.control = false),
            ("paused", |c| c.paused = true),
            ("locked", |c| c.locked = true),
            ("off screen", |c| c.on_screen = false),
            ("keyboard moved", |c| c.keyboard = Keyboard::Elsewhere),
            ("window gone", |c| {
                c.window = Rect::new(f64::NAN, 0.0, 0.0, -1.0);
            }),
        ];
        for (name, change) in cases {
            let mut gate = gate_with_frame();
            let before = context(10 * MS, &[]);
            posted(gate.handle(
                &before,
                &pointer(7, 0.5, 0.5, PointerAction::Down(Button::Left)),
            ));
            posted(gate.handle(&before, &key(4, true)));
            let mut after = context(5000 * MS, &sheet);
            change(&mut after);
            // A long time has passed, the window is covered, and the session has changed.
            assert!(
                matches!(
                    gate.handle(
                        &after,
                        &pointer(7, 0.9, 0.9, PointerAction::Up(Button::Left))
                    ),
                    Decision::Post(Post::Pointer {
                        action: PointerAction::Up(Button::Left),
                        ..
                    })
                ),
                "{name}: the button was not released"
            );
            assert!(
                matches!(
                    gate.handle(&after, &key(4, false)),
                    Decision::Post(Post::Key { down: false, .. })
                ),
                "{name}: the key was not released"
            );
            assert!(!gate.is_pressing(), "{name}");
        }
    }

    #[test]
    fn a_release_of_something_that_was_not_pressed_is_dropped() {
        let mut gate = gate_with_frame();
        let context = context(10 * MS, &[]);
        let up = pointer(7, 0.5, 0.5, PointerAction::Up(Button::Right));
        assert_eq!(dropped(gate.handle(&context, &up)), Reason::NotPressed);
        assert_eq!(
            dropped(gate.handle(&context, &key(4, false))),
            Reason::NotPressed
        );
        // The button that was pressed is not the one that was released.
        posted(gate.handle(
            &context,
            &pointer(7, 0.5, 0.5, PointerAction::Down(Button::Left)),
        ));
        assert_eq!(dropped(gate.handle(&context, &up)), Reason::NotPressed);
        assert!(gate.is_pressing());
    }

    #[test]
    fn a_drag_that_leaves_the_window_goes_on_at_the_edge_and_ends_there() {
        let mut gate = gate_with_frame();
        let context = context(10 * MS, &[]);
        posted(gate.handle(
            &context,
            &pointer(7, 0.9, 0.5, PointerAction::Down(Button::Left)),
        ));
        // The picture has no pixel beyond its edge, so the viewer's position is the edge, and
        // the spot is the window's far edge, one point inside.
        let beyond = pointer(7, 1.0, 0.5, PointerAction::Move);
        assert_eq!(
            posted(gate.handle(&context, &beyond)),
            at(899.0, 400.0, PointerAction::Move)
        );
        let up = pointer(7, 1.0, 0.5, PointerAction::Up(Button::Left));
        assert_eq!(
            posted(gate.handle(&context, &up)),
            at(899.0, 400.0, PointerAction::Up(Button::Left))
        );
        assert!(!gate.is_pressing());
    }

    #[test]
    fn a_drag_in_the_margin_of_the_picture_is_moved_to_the_window() {
        let mut gate = Gate::new();
        gate.record_frame(7, Duration::ZERO, geometry_with_margin());
        let context = context(10 * MS, &[]);
        posted(gate.handle(
            &context,
            &pointer(7, 0.5, 0.5, PointerAction::Down(Button::Left)),
        ));
        // Left of the content: the window's left edge.
        let margin = pointer(7, 0.02, 0.5, PointerAction::Move);
        assert_eq!(
            posted(gate.handle(&context, &margin)),
            at(100.0, 400.0, PointerAction::Move)
        );
        // Right of it: the far edge, one point inside.
        let right = pointer(7, 0.99, 0.5, PointerAction::Move);
        assert_eq!(
            posted(gate.handle(&context, &right)),
            at(899.0, 400.0, PointerAction::Move)
        );
    }

    #[test]
    fn positions_are_put_on_the_screen_relative_to_where_the_window_was_at_the_press() {
        let mut gate = gate_with_frame();
        // The press is on the title bar: 50 points across, 37.5 down. (Positions are chosen
        // that a float holds exactly.)
        let press = pointer(7, 0.0625, 0.0625, PointerAction::Down(Button::Left));
        assert_eq!(
            posted(gate.handle(&context(10 * MS, &[]), &press)),
            at(150.0, 137.5, PointerAction::Down(Button::Left))
        );
        // The window follows the pointer, so it is now 10 points to the right.
        let mut dragged = context(20 * MS, &[]);
        dragged.window = Rect::new(110.0, 100.0, 800.0, 600.0);
        // The finger moved to 100 points across. Relative to the window as it was, that is
        // (200, 137.5). Relative to the window as it is now it would be (210, 137.5), and the
        // pointer would run ahead of the window by the distance that the window has moved.
        let drag = pointer(7, 0.125, 0.0625, PointerAction::Move);
        assert_eq!(
            posted(gate.handle(&dragged, &drag)),
            at(200.0, 137.5, PointerAction::Move)
        );
        let up = pointer(7, 0.125, 0.0625, PointerAction::Up(Button::Left));
        assert_eq!(
            posted(gate.handle(&dragged, &up)),
            at(200.0, 137.5, PointerAction::Up(Button::Left))
        );
        // With nothing pressed the window is where it is.
        assert_eq!(
            posted(gate.handle(&dragged, &drag)),
            at(210.0, 137.5, PointerAction::Move)
        );
    }

    #[test]
    fn a_second_button_does_not_move_the_anchor_and_the_last_release_clears_it() {
        let mut gate = gate_with_frame();
        posted(gate.handle(
            &context(10 * MS, &[]),
            &pointer(7, 0.5, 0.5, PointerAction::Down(Button::Left)),
        ));
        let mut moved = context(20 * MS, &[]);
        moved.window = Rect::new(130.0, 100.0, 800.0, 600.0);
        // The second button goes down while the window is elsewhere: still the first position.
        assert_eq!(
            posted(gate.handle(
                &moved,
                &pointer(7, 0.5, 0.5, PointerAction::Down(Button::Right))
            )),
            at(500.0, 400.0, PointerAction::Down(Button::Right))
        );
        posted(gate.handle(
            &moved,
            &pointer(7, 0.5, 0.5, PointerAction::Up(Button::Left)),
        ));
        // One button is still down, so the anchor stays.
        assert_eq!(
            posted(gate.handle(&moved, &pointer(7, 0.5, 0.5, PointerAction::Move))),
            at(500.0, 400.0, PointerAction::Move)
        );
        posted(gate.handle(
            &moved,
            &pointer(7, 0.5, 0.5, PointerAction::Up(Button::Right)),
        ));
        assert_eq!(
            posted(gate.handle(&moved, &pointer(7, 0.5, 0.5, PointerAction::Move))),
            at(530.0, 400.0, PointerAction::Move)
        );
    }

    #[test]
    fn a_window_that_appears_during_a_drag_stops_the_moves_and_not_the_release() {
        let mut gate = gate_with_frame();
        posted(gate.handle(
            &context(10 * MS, &[]),
            &pointer(7, 0.25, 0.25, PointerAction::Down(Button::Left)),
        ));
        // A sheet appears over the right part of the window.
        let sheet = [other_app(Rect::new(400.0, 100.0, 300.0, 300.0))];
        let covered = context(20 * MS, &sheet);
        // Moves over the new window are dropped.
        let over = pointer(7, 0.5, 0.25, PointerAction::Move);
        assert_eq!(dropped(gate.handle(&covered, &over)), Reason::Covered);
        // The release is posted, and where the pointer was last put (the press, at 200 and 150
        // points into the window) and not over the sheet, which would receive the end of a
        // press that began elsewhere.
        let up = pointer(7, 0.5, 0.25, PointerAction::Up(Button::Left));
        assert_eq!(
            posted(gate.handle(&covered, &up)),
            at(300.0, 250.0, PointerAction::Up(Button::Left))
        );
        assert!(!gate.is_pressing());
    }

    #[test]
    fn a_release_goes_where_the_finger_lifted_if_nothing_is_in_front() {
        let mut gate = gate_with_frame();
        let context = context(10 * MS, &[]);
        posted(gate.handle(
            &context,
            &pointer(7, 0.25, 0.25, PointerAction::Down(Button::Left)),
        ));
        // The finger lifts somewhere the last move never reached.
        let up = pointer(7, 0.5, 0.5, PointerAction::Up(Button::Left));
        assert_eq!(
            posted(gate.handle(&context, &up)),
            at(500.0, 400.0, PointerAction::Up(Button::Left))
        );
    }

    #[test]
    fn a_release_after_the_window_changed_lands_inside_the_window_as_it_is_now() {
        let mut gate = gate_with_frame();
        posted(gate.handle(
            &context(10 * MS, &[]),
            &pointer(7, 0.875, 0.875, PointerAction::Down(Button::Left)),
        ));
        // The window is now smaller and elsewhere. The spot the pointer was put, 700 and 525
        // points into the old window, is outside the new one.
        let mut after = context(30 * MS, &[]);
        after.window = Rect::new(500.0, 500.0, 300.0, 200.0);
        let up = pointer(7, 0.1, 0.1, PointerAction::Up(Button::Left));
        let Post::Pointer { at, .. } = posted(gate.handle(&after, &up)) else {
            panic!("not a pointer event");
        };
        assert!(after.window.contains(at), "{at:?}");
        assert_eq!(at, Point { x: 799.0, y: 625.0 });
    }

    #[test]
    fn everything_that_was_pressed_is_released_when_delivery_stops() {
        let mut gate = gate_with_frame();
        let context = context(10 * MS, &[]);
        posted(gate.handle(
            &context,
            &pointer(7, 0.5, 0.5, PointerAction::Down(Button::Left)),
        ));
        posted(gate.handle(
            &context,
            &pointer(7, 0.5, 0.5, PointerAction::Down(Button::Right)),
        ));
        posted(gate.handle(&context, &key(4, true)));
        posted(gate.handle(&context, &key(55, true)));
        let posts = gate.release_all(window());
        assert_eq!(posts.len(), 4);
        for post in &posts {
            match post {
                Post::Pointer {
                    at,
                    action: PointerAction::Up(_),
                    ..
                } => assert!(window().contains(*at), "{at:?}"),
                Post::Key { down: false, .. } => {}
                other => panic!("{other:?}"),
            }
        }
        assert!(!gate.is_pressing());
        assert_eq!(gate.release_all(window()), [] as [Post; 0]);
        // And the anchor is gone: the next pointer event is relative to the window as it is.
        let mut moved = super::tests::context(20 * MS, &[]);
        moved.window = Rect::new(150.0, 100.0, 800.0, 600.0);
        assert_eq!(
            posted(gate.handle(&moved, &pointer(7, 0.5, 0.5, PointerAction::Move))),
            at(550.0, 400.0, PointerAction::Move)
        );
    }

    #[test]
    fn a_window_that_cannot_be_used_never_makes_the_gate_panic() {
        let bad = [
            Rect::new(f64::NAN, 0.0, 800.0, 600.0),
            Rect::new(0.0, 0.0, f64::INFINITY, 600.0),
            Rect::new(0.0, 0.0, -5.0, 600.0),
            Rect::new(0.0, 0.0, 0.0, 0.0),
        ];
        for window in bad {
            let mut gate = gate_with_frame();
            posted(gate.handle(
                &context(10 * MS, &[]),
                &pointer(7, 0.5, 0.5, PointerAction::Down(Button::Left)),
            ));
            let mut context = context(20 * MS, &[]);
            context.window = window;
            assert_eq!(
                dropped(gate.handle(&context, &pointer(7, 0.5, 0.5, PointerAction::Move))),
                Reason::NotOnScreen,
                "{window:?}"
            );
            assert_eq!(
                dropped(gate.handle(&context, &key(4, true))),
                Reason::NotOnScreen
            );
            assert!(matches!(
                gate.handle(
                    &context,
                    &pointer(7, 0.5, 0.5, PointerAction::Up(Button::Left))
                ),
                Decision::Post(_)
            ));
            posted(gate.handle(&super::tests::context(30 * MS, &[]), &key(4, true)));
            assert_eq!(gate.release_all(window).len(), 1);
        }
    }

    #[test]
    fn the_viewer_cannot_send_more_than_its_rate() {
        let mut gate = gate_with_frame();
        let context = context(10 * MS, &[]);
        posted(gate.handle(
            &context,
            &pointer(7, 0.5, 0.5, PointerAction::Down(Button::Left)),
        ));
        let event = pointer(7, 0.5, 0.5, PointerAction::Move);
        for _ in 1..MAX_EVENTS_PER_SECOND {
            posted(gate.handle(&context, &event));
        }
        assert_eq!(dropped(gate.handle(&context, &event)), Reason::TooFast);
        assert_eq!(
            dropped(gate.handle(&context, &key(4, true))),
            Reason::TooFast
        );
        // A release is never dropped for being too fast.
        assert!(matches!(
            gate.handle(
                &context,
                &pointer(7, 0.5, 0.5, PointerAction::Up(Button::Left))
            ),
            Decision::Post(_)
        ));
        // And the rate is a rate: a second later there is room again.
        let later = super::tests::context(10 * MS + Duration::from_secs(1), &[]);
        assert!(matches!(gate.handle(&later, &event), Decision::Post(_)));
    }

    #[test]
    fn a_viewer_cannot_hold_down_more_keys_than_a_person_has_fingers() {
        let mut gate = gate_with_frame();
        let context = context(10 * MS, &[]);
        for code in 0..u16::try_from(MAX_HELD_KEYS).unwrap() {
            posted(gate.handle(&context, &key(code, true)));
        }
        assert_eq!(
            dropped(gate.handle(&context, &key(1000, true))),
            Reason::TooManyHeld
        );
        // A key that is held already is a repeat, not another key.
        posted(gate.handle(&context, &key(3, true)));
    }

    #[test]
    fn a_geometry_that_cannot_be_used_is_not_remembered() {
        let mut gate = Gate::new();
        let mut bad = geometry();
        bad.content.size.width = 0.0;
        gate.record_frame(7, Duration::ZERO, bad);
        let mut bad = geometry();
        bad.window.height = f64::NAN;
        gate.record_frame(8, Duration::ZERO, bad);
        let context = context(10 * MS, &[]);
        for frame in [7, 8] {
            let event = pointer(frame, 0.5, 0.5, PointerAction::Move);
            assert_eq!(dropped(gate.handle(&context, &event)), Reason::FrameUnknown);
        }
    }

    #[test]
    fn an_event_that_breaks_the_limits_is_dropped_and_changes_nothing() {
        let mut gate = gate_with_frame();
        let context = context(10 * MS, &[]);
        let bad = [
            pointer(7, f32::NAN, 0.5, PointerAction::Down(Button::Left)),
            pointer(7, 2.0, 0.5, PointerAction::Down(Button::Left)),
            pointer(
                7,
                0.5,
                0.5,
                PointerAction::Scroll {
                    dx: f32::INFINITY,
                    dy: 0.0,
                },
            ),
            Event::Key {
                code: 4,
                down: true,
                modifiers: Modifiers::NONE,
                repeat: false,
                text: Some("a".repeat(100)),
            },
        ];
        for event in &bad {
            assert_eq!(dropped(gate.handle(&context, event)), Reason::Malformed);
        }
        assert!(!gate.is_pressing());
        assert!(gate.recent.is_empty());
    }

    #[test]
    fn at_most_one_notice_a_second_and_it_names_the_reason_that_dropped_most() {
        let mut notices = Notices::new();
        assert_eq!(notices.poll(Duration::ZERO), None);
        notices.dropped(Reason::Covered);
        notices.dropped(Reason::NoKeyboard);
        notices.dropped(Reason::NoKeyboard);
        assert_eq!(
            notices.poll(Duration::ZERO),
            Some(Notice {
                reason: Reason::NoKeyboard,
                count: 2
            })
        );
        // The next one waits for a second, and the counts start again.
        notices.dropped(Reason::Covered);
        assert_eq!(notices.poll(999 * MS), None);
        assert_eq!(
            notices.poll(1000 * MS),
            Some(Notice {
                reason: Reason::Covered,
                count: 1
            })
        );
        assert_eq!(notices.poll(5000 * MS), None);
    }

    #[test]
    fn a_post_does_not_print_what_was_typed() {
        let post = Post::Key {
            code: 35,
            down: true,
            modifiers: Modifiers::NONE,
            repeat: false,
            text: Some("hunter2".into()),
        };
        let shown = format!("{post:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("text_bytes: Some(7)"), "{shown}");
    }
    /// One step of a session, as the platform and the viewer would produce it.
    #[derive(Debug, Clone)]
    // The facts of a session are independent of each other, and the test wants every mix.
    #[allow(clippy::struct_excessive_bools)]
    struct Step {
        advance_ms: u64,
        control: bool,
        paused: bool,
        locked: bool,
        on_screen: bool,
        keyboard: Keyboard,
        fronts: Vec<Front>,
        window: Rect,
        paint: Paint,
        frames_back: u64,
        shape: Shape,
    }

    #[derive(Debug, Clone, Copy)]
    enum Paint {
        /// The window did not change.
        Nothing,
        /// A new picture, which is sent as the next frame.
        Frame,
        /// A new picture that is dropped before the queue.
        Dropped,
    }

    #[derive(Debug, Clone)]
    enum Shape {
        Pointer(f32, f32, PointerAction),
        Key(u16, bool),
    }

    fn step() -> impl proptest::strategy::Strategy<Value = Step> {
        use proptest::prelude::*;
        let button = prop_oneof![
            Just(Button::Left),
            Just(Button::Right),
            Just(Button::Middle)
        ];
        let action = prop_oneof![
            4 => Just(PointerAction::Move),
            2 => button.clone().prop_map(PointerAction::Down),
            2 => button.prop_map(PointerAction::Up),
            1 => (-50.0_f32..50.0, -50.0_f32..50.0)
                .prop_map(|(dx, dy)| PointerAction::Scroll { dx, dy }),
        ];
        let shape = prop_oneof![
            3 => (0.0_f32..=1.0, 0.0_f32..=1.0, action)
                .prop_map(|(x, y, action)| Shape::Pointer(x, y, action)),
            1 => (0_u16..40, any::<bool>()).prop_map(|(code, down)| Shape::Key(code, down)),
        ];
        let keyboard = prop_oneof![
            8 => Just(Keyboard::Shared),
            1 => Just(Keyboard::Elsewhere),
            1 => Just(Keyboard::Unknown),
        ];
        let front = (
            0.0_f64..1000.0,
            0.0_f64..800.0,
            1.0_f64..400.0,
            1.0_f64..400.0,
            any::<bool>(),
            any::<bool>(),
        )
            .prop_map(|(x, y, w, h, same_application, menu)| Front {
                rect: Rect::new(x, y, w, h),
                same_application,
                menu,
            });
        // The window stays, moves, or changes its size.
        let window = prop_oneof![
            6 => Just(window()),
            2 => Just(Rect::new(130.0, 90.0, 800.0, 600.0)),
            1 => Just(Rect::new(100.0, 100.0, 700.0, 600.0)),
        ];
        let paint = prop_oneof![
            3 => Just(Paint::Nothing),
            4 => Just(Paint::Frame),
            1 => Just(Paint::Dropped),
        ];
        let advance = prop_oneof![8 => 1_u64..120, 2 => 400_u64..1500];
        // The session is mostly fine: the rules are most interesting when they let things pass.
        let usually = |n: u32| prop_oneof![n => Just(true), 1 => Just(false)];
        (
            (
                advance,
                usually(9),
                usually(9).prop_map(|b| !b),
                usually(9).prop_map(|b| !b),
            ),
            (
                usually(9),
                keyboard,
                proptest::collection::vec(front, 0..3),
                window,
            ),
            (paint, 0_u64..4, shape),
        )
            .prop_map(
                |(
                    (advance_ms, control, paused, locked),
                    (on_screen, keyboard, fronts, window),
                    (paint, frames_back, shape),
                )| Step {
                    advance_ms,
                    control,
                    paused,
                    locked,
                    on_screen,
                    keyboard,
                    fronts,
                    window,
                    paint,
                    frames_back,
                    shape,
                },
            )
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]

        /// Whatever the viewer sends and whatever the Mac is doing: what is posted obeys the
        /// rules, judged by a model that keeps its own record of frames and changes; a plain
        /// move on a current picture is always posted; and when delivery stops everything that
        /// was pressed is released and nothing else.
        #[test]
        fn what_is_posted_obeys_the_rules_and_every_press_is_released(
            steps in proptest::collection::vec(step(), 1..100),
        ) {
            let mut gate = Gate::new();
            // The model: every frame sent and every change of the picture, with their times.
            let mut frames: Vec<(u64, Duration)> = Vec::new();
            let mut changes: Vec<Duration> = Vec::new();
            let mut number = 0_u64;
            let mut now = Duration::ZERO;
            let mut buttons: BTreeSet<Button> = BTreeSet::new();
            let mut keys: BTreeSet<u16> = BTreeSet::new();
            let mut anchor: Option<Rect> = None;
            for step in steps {
                now += Duration::from_millis(step.advance_ms);
                match step.paint {
                    Paint::Nothing => {}
                    Paint::Frame => {
                        number += 1;
                        gate.record_frame(number, now, geometry());
                        frames.push((number, now));
                        changes.push(now);
                    }
                    Paint::Dropped => {
                        gate.record_change(now);
                        changes.push(now);
                    }
                }
                let context = Context {
                    now,
                    control: step.control,
                    paused: step.paused,
                    locked: step.locked,
                    on_screen: step.on_screen,
                    window: step.window,
                    in_front: &step.fronts,
                    keyboard: step.keyboard,
                };
                let session_ok =
                    context.control && !context.paused && !context.locked && context.on_screen;
                let named = number.saturating_sub(step.frames_back);
                let known = frames.iter().rev().find(|(n, _)| *n == named).copied();
                // When the picture of a frame went out of date, by the model's own record.
                let out_of_date =
                    |picture: Duration| changes.iter().copied().filter(|c| *c > picture).min();
                let current_enough = |picture: Duration| {
                    out_of_date(picture).is_none_or(|at| now.saturating_sub(at) <= 500 * MS)
                };
                let same_size = (context.window.size.width - 800.0).abs() <= 0.5
                    && (context.window.size.height - 600.0).abs() <= 0.5;
                let event = match &step.shape {
                    Shape::Pointer(x, y, action) => pointer(named, *x, *y, *action),
                    Shape::Key(code, down) => key(*code, *down),
                };
                let was_dragging = !buttons.is_empty();
                let decision = gate.handle(&context, &event);
                match &decision {
                    Decision::Post(Post::Pointer { at, action, .. }) => {
                        let covered_now = step.fronts.iter().any(|f| f.rect.contains(*at));
                        if let PointerAction::Up(button) = action {
                            // A release is always delivered, inside the window as it is now.
                            proptest::prop_assert!(buttons.remove(button));
                            proptest::prop_assert!(context.window.contains(*at), "{at:?}");
                            if buttons.is_empty() {
                                anchor = None;
                            }
                        } else {
                            proptest::prop_assert!(session_ok);
                            proptest::prop_assert!(!covered_now, "{at:?}");
                            let (_, picture) = known.expect("a frame that is known");
                            proptest::prop_assert!(current_enough(picture));
                            proptest::prop_assert!(same_size);
                            // Inside the window, or, in a drag, inside the window as it was at
                            // the press.
                            let home = if was_dragging { anchor.unwrap() } else { context.window };
                            proptest::prop_assert!(home.contains(*at), "{at:?} {home:?}");
                            if let PointerAction::Down(button) = action {
                                if buttons.is_empty() {
                                    anchor = Some(context.window);
                                }
                                buttons.insert(*button);
                            }
                        }
                    }
                    Decision::Post(Post::Key { code, down, .. }) => {
                        if *down {
                            proptest::prop_assert!(session_ok);
                            proptest::prop_assert_eq!(context.keyboard, Keyboard::Shared);
                            let sibling_over = step.fronts.iter().any(|f| {
                                f.same_application && (f.menu || f.rect.overlaps(&context.window))
                            });
                            proptest::prop_assert!(!sibling_over);
                            keys.insert(*code);
                        } else {
                            proptest::prop_assert!(keys.remove(code));
                        }
                    }
                    Decision::Drop(_) => {
                        // Liveness: a plain move that nothing forbids is posted.
                        let plain_move = matches!(
                            step.shape,
                            Shape::Pointer(_, _, PointerAction::Move)
                        );
                        if plain_move
                            && session_ok
                            && step.fronts.is_empty()
                            && same_size
                            && let Some((_, picture)) = known
                        {
                            proptest::prop_assert!(
                                !current_enough(picture),
                                "a move on a current picture was dropped: {decision:?}"
                            );
                        }
                    }
                }
                proptest::prop_assert_eq!(gate.is_pressing(), !buttons.is_empty() || !keys.is_empty());
            }
            let posts = gate.release_all(window());
            let mut released_buttons = BTreeSet::new();
            let mut released_keys = BTreeSet::new();
            for post in posts {
                match post {
                    Post::Pointer { at, action: PointerAction::Up(button), .. } => {
                        proptest::prop_assert!(window().contains(at), "{at:?}");
                        released_buttons.insert(button);
                    }
                    Post::Key { code, down: false, .. } => {
                        released_keys.insert(code);
                    }
                    other => proptest::prop_assert!(false, "{other:?}"),
                }
            }
            proptest::prop_assert_eq!(released_buttons, buttons);
            proptest::prop_assert_eq!(released_keys, keys);
            proptest::prop_assert!(!gate.is_pressing());
        }
    }
}
