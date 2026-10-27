// SPDX-License-Identifier: Apache-2.0

//! A platform that shares windows, played by the tests through the same C functions that a real
//! one provides: the counterpart of [`Fake`](crate::fake::Fake), which shows them.
//!
//! It logs every call it gets, checks each one against what the header promises (sizes, pointers,
//! that a window is called only after the person picked it and never again after `release`, that
//! `prepare` and `start` come once and in that order, that `ended` follows only a `start` that
//! worked, that `release` comes once and last, and that the calls about one window never overlap)
//! and writes down a complaint for each promise that is broken. A test holds the Rust side to the
//! header with [`FakeSharer::complaints`], and makes the platform fail, lie or call back into the
//! library with the settings.
//!
//! A window that is shared with control also gets `surroundings` and `post`: the first answers
//! with what the test says the Mac is doing ([`Around`]), the second checks the struct against
//! the header and records what it says as the [`Post`] that the gate made, so that a test can
//! compare it with what it expects. A window that was given as private also gets `lift`, which
//! checks that the window was given so and that only the bits of the header are set.
//!
//! A display that is shared with sound also gets `hear`, which says whether the other device wants
//! to hear it and is logged as [`SharerCall::Hear`]. The fake records it and does nothing else: a
//! test plays the encoder of the sound with [`sound_begin`], [`sound`] and [`sound_silent`], the
//! way the capture thread of a Mac would.
//!
//! A window that was given as private also gets `leave`, which is `ended` for a session that the
//! person at the other device ended by choosing that the Mac stay as it is. It is logged as
//! [`SharerCall::Leave`] and counts as the end of the window: nothing may come after it, and
//! neither `ended` nor a second `leave` may follow.
//!
//! The other half of a platform, the capture thread that asks `begin` and hands over what the
//! encoder made, is a set of functions that call the library the way Swift would, with the same
//! structs and pointers: [`begin`], [`sent`] and the others.

#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    collections::BTreeSet,
    ffi::c_void,
    mem::offset_of,
    ptr, slice,
    sync::{
        Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use baton_stream::{
    gate::{Point, Post},
    input::{Button, MAX_KEY_CODE, Modifiers, PointerAction},
};

use crate::{
    Error, Registry,
    abi::{
        ABI_VERSION, BUTTON_LEFT, BUTTON_MIDDLE, BUTTON_RIGHT, BatonBytes, BatonFormat, BatonFront,
        BatonGeometry, BatonMessage, BatonPicked, BatonPicture, BatonPost, BatonRect,
        BatonSharerVTable, BatonSound, BatonSoundPacket, BatonSurroundings, BatonTakes, CODEC_H264,
        FRONTS_MAX, KEYBOARD_SHARED, KEYBOARD_UNKNOWN, LIFT_INPUT, LIFT_SCREEN, MESSAGE_MAX, OK,
        PICKED_DISPLAY, PICKED_WINDOW, POINTER_DOWN, POINTER_MOVE, POINTER_SCROLL, POINTER_UP,
        POST_KEY, POST_POINTER, SCREEN_PRIVATE, SOUND_CODEC_AAC, size_of_u32,
    },
    platform::SharerCalls,
};

/// What the platform got, as a test can compare it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SharerCall {
    Control,
    Pick {
        request: u64,
    },
    CancelPick {
        request: u64,
    },
    /// The platform was asked for its main display, with these flags.
    Screen {
        flags: u32,
    },
    /// What the other device takes: the widest and the highest picture, the rate and the codecs.
    Prepare {
        takes: (u32, u32, u32, u32),
    },
    Start {
        link: u64,
    },
    /// The platform was asked what the Mac is doing around the window.
    Surroundings,
    /// The platform was told to put this to the window: what the struct said, as the rules made
    /// it.
    Post(Post),
    /// The platform was told that this is lifted of the privacy of the screen now: `LIFT_` bits.
    Lift {
        lifted: u32,
    },
    /// The platform was told whether the sound of the screen is wanted.
    Hear {
        on: bool,
    },
    Ended,
    /// The platform was told that the session is over and that the person at the other device
    /// chose that the Mac stay as it is: the end of the window, in place of [`Self::Ended`].
    Leave,
    Release,
}

/// The platform was told that the other device wants to hear the sound.
pub(crate) const RECORD: SharerCall = SharerCall::Hear { on: true };

/// The platform was told that the other device does not want to hear the sound.
pub(crate) const STOP: SharerCall = SharerCall::Hear { on: false };

/// What `pick` does.
#[derive(Debug, Clone)]
pub(crate) enum PickPlan {
    /// Works, and the test answers with [`FakeSharer::answer`] when it likes.
    Later,
    /// Fails with `code`. It writes `text` (as far as there is room) and says that it wrote
    /// `claimed_len` bytes, which a platform that is wrong about it can make more than there was
    /// room for. A `code` of `OK` is a pick that works and leaves a text behind all the same.
    Fail {
        code: i32,
        text: Vec<u8>,
        claimed_len: u32,
    },
}

/// What `screen` does.
#[derive(Debug, Clone)]
pub(crate) enum ScreenPlan {
    /// Gives a display of this size, a new window of the fake.
    Gives(u32, u32),
    /// Returns this code, writing `text` as `PickPlan::Fail` does. It gives nothing.
    Fails {
        code: i32,
        text: Vec<u8>,
        claimed_len: u32,
    },
    /// Returns `OK` and writes these numbers as they are, which a platform that is wrong about
    /// the header may do: what it says it gave, the size, and whether it gave a window of its own
    /// (a pointer that is not null).
    Writes {
        kind: u32,
        width: u32,
        height: u32,
        window: bool,
    },
    /// Does not return until the test lets it, and then gives a display of this size.
    Waits(u32, u32),
}

/// What `prepare` does.
#[derive(Debug, Clone, Copy)]
pub(crate) enum PreparePlan {
    /// The picture of the encoder fixture (64 by 48), or less if the other device takes less, at
    /// 30 frames a second or less.
    Within,
    /// Chooses this, whatever the other device takes: the codec, the width, the height and the
    /// rate.
    Format(u32, u32, u32, u32),
    /// Fails with this code, having written nothing.
    Fail(i32),
    /// Chooses as `Within` does, and answers with this code: a number other than `OK` is a
    /// failure whatever was written.
    Answers(i32),
}

/// What `start` does.
#[derive(Debug, Clone)]
pub(crate) enum StartPlan {
    Works,
    /// Fails with `code`, writing `text` as `PickPlan::Fail` does.
    Fail {
        code: i32,
        text: Vec<u8>,
        claimed_len: u32,
    },
}

/// What the platform says the Mac is doing around the window.
#[derive(Debug, Clone)]
pub(crate) struct Around {
    pub(crate) locked: bool,
    pub(crate) on_screen: bool,
    /// A `KEYBOARD_` number.
    pub(crate) keyboard: u32,
    pub(crate) window: BatonRect,
    pub(crate) fronts: Vec<BatonFront>,
}

impl Default for Around {
    /// A window of 400 by 300 points at (100, 50), on screen, with the keyboard, and nothing in
    /// front of it.
    fn default() -> Self {
        Self {
            locked: false,
            on_screen: true,
            keyboard: KEYBOARD_SHARED,
            window: BatonRect {
                x: 100.0,
                y: 50.0,
                width: 400.0,
                height: 300.0,
            },
            fronts: Vec::new(),
        }
    }
}

/// The post of the left button going down at this point of the screen, with no modifier key.
pub(crate) fn left_down(x: f64, y: f64) -> Post {
    pointer_post(x, y, PointerAction::Down(Button::Left))
}

/// The post of the left button coming up at this point of the screen, with no modifier key.
pub(crate) fn left_up(x: f64, y: f64) -> Post {
    pointer_post(x, y, PointerAction::Up(Button::Left))
}

/// The post of a pointer at this point of the screen, with no modifier key.
pub(crate) fn pointer_post(x: f64, y: f64, action: PointerAction) -> Post {
    Post::Pointer {
        at: Point { x, y },
        modifiers: Modifiers::NONE,
        action,
    }
}

/// The post of a key going down or up, with no modifier key, and not a repeat.
pub(crate) fn key_post(code: u16, down: bool, text: Option<&str>) -> Post {
    Post::Key {
        code,
        down,
        modifiers: Modifiers::NONE,
        repeat: false,
        text: text.map(str::to_owned),
    }
}

/// What `surroundings` does.
#[derive(Debug, Clone)]
pub(crate) enum AroundPlan {
    /// Answers with this.
    Says(Around),
    /// Fails with this code, having written nothing.
    Fails(i32),
    /// Works, and writes nothing: what the library put in the struct is what it gets back.
    WritesNothing,
    /// Answers with this, and claims this many windows in front, whatever there is room for.
    Claims(Around, u32),
    /// Writes these numbers as they are, which a platform that is wrong about the header may do:
    /// whether the Mac is locked, whether the window is on the screen, and who has the keyboard.
    /// The window is that of [`Around::default`], and nothing is in front of it.
    Numbers {
        locked: u32,
        on_screen: u32,
        keyboard: u32,
    },
}

/// Called inside a call of the platform, with the call and the number of the window it is about
/// (0 for the calls about the device), to make the platform call the library.
pub(crate) type Hook = Arc<dyn Fn(u64, &SharerCall) + Send + Sync>;

#[allow(clippy::struct_excessive_bools)] // the switches of one platform, each of its own
struct State {
    log: Vec<(u64, SharerCall)>,
    complaints: Vec<String>,
    control: u32,
    pick: PickPlan,
    screen: ScreenPlan,
    /// Whether a `screen` that waits may go on.
    screen_open: bool,
    prepare: PreparePlan,
    start: StartPlan,
    around: AroundPlan,
    /// What `post` answers: `OK`, or a failure.
    post: i32,
    /// The platform registers a table that ends before `lift`.
    cannot_lift: bool,
    /// The platform registers a table that ends before `leave`.
    cannot_leave: bool,
    /// Whether the platform registers as one that sends sound.
    sound: bool,
    hook: Option<Hook>,
    /// The requests of picks that wait for an answer.
    waiting: BTreeSet<u64>,
}

#[derive(Default)]
#[allow(clippy::struct_excessive_bools)] // the facts about one window, each of its own
struct Progress {
    prepared: bool,
    started: bool,
    start_failed: bool,
    ended: bool,
    released: bool,
    /// The window stayed with the platform: the library refused the answer that gave it, so it
    /// may not be called.
    kept: bool,
    /// The window is a display that was given with `SCREEN_PRIVATE`.
    private: bool,
    link: u64,
}

/// One window of the fake: what the pointer that an answer gave stands for.
struct Cell {
    fake: Weak<FakeSharer>,
    number: u64,
    progress: Mutex<Progress>,
    /// Whether a call about this window is under way.
    busy: AtomicBool,
}

/// The platform of a test that shares windows.
pub(crate) struct FakeSharer {
    state: Mutex<State>,
    /// Told when `screen_open` changes.
    screen_gate: Condvar,
    /// The windows that were given. They stay until the fake is dropped, so a call that comes
    /// after `release` finds the flag and not freed memory.
    cells: Mutex<Vec<Arc<Cell>>>,
}

/// Every fake that was made, so that none is dropped while a window of it is still alive.
static KEPT: Mutex<Vec<Arc<FakeSharer>>> = Mutex::new(Vec::new());

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Registers `table` in `registry` as a platform does.
pub(crate) fn register_table(registry: &Registry, table: &BatonSharerVTable) -> Result<(), Error> {
    // SAFETY: `table` is a struct of the interface that is whole and that nothing writes to
    // meanwhile; its context is the fake's, which lives for the whole test.
    unsafe { registry.register_sharer(table) }
}

/// The calls that registration makes of `table`.
pub(crate) fn read_table(table: &BatonSharerVTable) -> Result<SharerCalls, Error> {
    // SAFETY: as above.
    unsafe { SharerCalls::read(table) }
}

impl FakeSharer {
    /// A platform that cannot let another device control a window, and works whenever it is
    /// asked to.
    ///
    /// It lives for as long as the process does: a vtable holds its address and not a reference,
    /// and a session that is still ending when a test returns must find it where it was.
    pub(crate) fn new() -> Arc<Self> {
        let fake = Arc::new(Self {
            screen_gate: Condvar::new(),
            state: Mutex::new(State {
                log: Vec::new(),
                complaints: Vec::new(),
                control: 0,
                pick: PickPlan::Later,
                screen: ScreenPlan::Gives(crate::fixture::WIDTH, crate::fixture::HEIGHT),
                screen_open: false,
                prepare: PreparePlan::Within,
                start: StartPlan::Works,
                around: AroundPlan::Says(Around::default()),
                post: OK,
                cannot_lift: false,
                cannot_leave: false,
                sound: true,
                hook: None,
                waiting: BTreeSet::new(),
            }),
            cells: Mutex::new(Vec::new()),
        });
        locked(&KEPT).push(Arc::clone(&fake));
        fake
    }

    /// The vtable that registers this platform.
    pub(crate) fn vtable(self: &Arc<Self>) -> BatonSharerVTable {
        BatonSharerVTable {
            version: ABI_VERSION,
            size: size_of_u32::<BatonSharerVTable>(),
            context: Arc::as_ptr(self).cast_mut().cast(),
            control: Some(control),
            pick: Some(pick),
            cancel_pick: Some(cancel_pick),
            prepare: Some(prepare),
            start: Some(start),
            ended: Some(ended),
            release: Some(release),
            surroundings: Some(surroundings),
            post: Some(post),
            screen: Some(screen),
            lift: Some(lift),
            hear: Some(hear),
            leave: Some(leave),
        }
    }

    /// The vtable of a platform that was written before input was carried: it ends at `release`,
    /// and the library reads the four functions that it lacks as missing.
    pub(crate) fn vtable_without_input(self: &Arc<Self>) -> BatonSharerVTable {
        BatonSharerVTable {
            size: u32::try_from(offset_of!(BatonSharerVTable, surroundings)).unwrap(),
            surroundings: None,
            post: None,
            screen: None,
            lift: None,
            hear: None,
            leave: None,
            ..self.vtable()
        }
    }

    /// The vtable of a platform that was written before it could give its screen: it ends at
    /// `post`, and the library reads the two functions that it lacks as missing.
    pub(crate) fn vtable_without_screen(self: &Arc<Self>) -> BatonSharerVTable {
        BatonSharerVTable {
            size: u32::try_from(offset_of!(BatonSharerVTable, screen)).unwrap(),
            screen: None,
            lift: None,
            hear: None,
            leave: None,
            ..self.vtable()
        }
    }

    /// The vtable of a platform that was written before the privacy of a screen could be
    /// lifted: it ends at `screen`, and the library reads the two functions that it lacks as
    /// missing.
    pub(crate) fn vtable_without_lift(self: &Arc<Self>) -> BatonSharerVTable {
        BatonSharerVTable {
            size: u32::try_from(offset_of!(BatonSharerVTable, lift)).unwrap(),
            lift: None,
            hear: None,
            leave: None,
            ..self.vtable()
        }
    }

    /// The vtable of a platform that was written before it could send sound: it ends at `lift`,
    /// and the library reads the two functions that it lacks as missing.
    pub(crate) fn vtable_without_hear(self: &Arc<Self>) -> BatonSharerVTable {
        BatonSharerVTable {
            size: u32::try_from(offset_of!(BatonSharerVTable, hear)).unwrap(),
            hear: None,
            leave: None,
            ..self.vtable()
        }
    }

    /// The vtable of a platform that was written before it could leave the Mac as it is at the
    /// end of a private session: it ends at `hear`, and the library reads the function that it
    /// lacks as missing.
    pub(crate) fn vtable_without_leave(self: &Arc<Self>) -> BatonSharerVTable {
        BatonSharerVTable {
            size: u32::try_from(offset_of!(BatonSharerVTable, leave)).unwrap(),
            leave: None,
            ..self.vtable()
        }
    }

    /// Registers this platform in `registry`, as one that sends no sound.
    pub(crate) fn register_without_hear_in(
        self: &Arc<Self>,
        registry: &Registry,
    ) -> Result<(), Error> {
        register_table(registry, &self.vtable_without_hear())
    }

    /// Registers this platform in `registry`: with the whole table, or with the one that ends
    /// before `lift` if the test said that the platform cannot lift, or the one that ends before
    /// `hear` if it said that the platform sends no sound ([`Self::sends_no_sound`]), or the one
    /// that ends before `leave` if it said that the platform cannot leave the Mac as it is
    /// ([`Self::set_cannot_leave`]).
    pub(crate) fn register_in(self: &Arc<Self>, registry: &Registry) -> Result<(), Error> {
        register_table(registry, &self.registered_table())
    }

    /// The calls that registration makes of this platform, with the table that
    /// [`register_in`](Self::register_in) registers.
    pub(crate) fn platform(self: &Arc<Self>) -> SharerCalls {
        read_table(&self.registered_table()).expect("the vtable of the fake is whole")
    }

    /// The table that registers this platform: the whole one, or the one of an older platform if
    /// the test said that this one is.
    fn registered_table(self: &Arc<Self>) -> BatonSharerVTable {
        if self.state().cannot_lift {
            self.vtable_without_lift()
        } else if !self.state().sound {
            self.vtable_without_hear()
        } else if self.state().cannot_leave {
            self.vtable_without_leave()
        } else {
            self.vtable()
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        locked(&self.state)
    }

    /// Records a call, and runs the hook, if there is one, with nothing locked.
    fn heard(&self, window: u64, call: &SharerCall) {
        let hook = {
            let mut state = self.state();
            state.log.push((window, call.clone()));
            state.hook.clone()
        };
        if let Some(hook) = hook {
            hook(window, call);
        }
    }

    fn complain(&self, what: impl Into<String>) {
        self.state().complaints.push(what.into());
    }

    // What the tests set.

    /// Whether the person can let another device control a window.
    pub(crate) fn set_control(&self, control: bool) {
        self.state().control = u32::from(control);
    }

    /// The number that `control` answers with, for a platform that says something else than 1
    /// or 0.
    pub(crate) fn set_control_number(&self, number: u32) {
        self.state().control = number;
    }

    /// The platform registers as one that was written before it could send sound.
    pub(crate) fn sends_no_sound(&self) {
        self.state().sound = false;
    }

    pub(crate) fn plan_pick(&self, plan: PickPlan) {
        self.state().pick = plan;
    }

    pub(crate) fn plan_screen(&self, plan: ScreenPlan) {
        self.state().screen = plan;
    }

    /// Lets a `screen` that waits go on, now and for the calls after it.
    pub(crate) fn open_screen(&self) {
        self.state().screen_open = true;
        self.screen_gate.notify_all();
    }

    pub(crate) fn plan_prepare(&self, plan: PreparePlan) {
        self.state().prepare = plan;
    }

    pub(crate) fn plan_start(&self, plan: StartPlan) {
        self.state().start = plan;
    }

    /// What the Mac is doing from now on.
    pub(crate) fn plan_around(&self, plan: AroundPlan) {
        self.state().around = plan;
    }

    /// The Mac is doing this from now on.
    pub(crate) fn set_around(&self, around: Around) {
        self.plan_around(AroundPlan::Says(around));
    }

    /// What `post` answers from now on.
    pub(crate) fn plan_post(&self, code: i32) {
        self.state().post = code;
    }

    /// The platform registers a table that ends at `screen`, as one written before the privacy
    /// of a screen could be lifted does.
    pub(crate) fn set_cannot_lift(&self) {
        self.state().cannot_lift = true;
    }

    /// The platform registers a table that ends at `hear`, as one written before a private
    /// session could end without the lock does.
    pub(crate) fn set_cannot_leave(&self) {
        self.state().cannot_leave = true;
    }

    pub(crate) fn on_call(&self, hook: impl Fn(u64, &SharerCall) + Send + Sync + 'static) {
        self.state().hook = Some(Arc::new(hook));
    }

    // What the tests read.

    /// Every call, in order, with the number of the window it was about (0 for the calls about
    /// the device: the first window is 1).
    pub(crate) fn log(&self) -> Vec<(u64, SharerCall)> {
        self.state().log.clone()
    }

    /// What the window was told to do, in order.
    pub(crate) fn posts(&self, window: u64) -> Vec<Post> {
        self.calls(window)
            .into_iter()
            .filter_map(|call| match call {
                SharerCall::Post(post) => Some(post),
                _ => None,
            })
            .collect()
    }

    /// What the window was told about the sound, in order: [`RECORD`] and [`STOP`].
    pub(crate) fn hear_calls(&self, window: u64) -> Vec<SharerCall> {
        let mut calls = self.calls(window);
        calls.retain(|call| matches!(call, SharerCall::Hear { .. }));
        calls
    }

    /// The calls about one window, in order.
    pub(crate) fn calls(&self, window: u64) -> Vec<SharerCall> {
        self.state()
            .log
            .iter()
            .filter(|(of, _)| *of == window)
            .map(|(_, call)| call.clone())
            .collect()
    }

    /// How many calls of this kind there were, over all windows.
    pub(crate) fn count(&self, kind: impl Fn(&SharerCall) -> bool) -> usize {
        self.state()
            .log
            .iter()
            .filter(|(_, call)| kind(call))
            .count()
    }

    /// The promises of the header that the Rust side broke. Empty when it kept them all.
    pub(crate) fn complaints(&self) -> Vec<String> {
        self.state().complaints.clone()
    }

    /// Fails the test with the promises of the header that the Rust side broke, if it broke any.
    #[track_caller]
    pub(crate) fn assert_no_complaints(&self) {
        let complaints = self.complaints();
        assert!(complaints.is_empty(), "{complaints:?}");
    }

    /// The requests of the picks that were not answered or taken down.
    pub(crate) fn waiting(&self) -> Vec<u64> {
        self.state().waiting.iter().copied().collect()
    }

    /// The request of the newest pick.
    pub(crate) fn last_request(&self) -> Option<u64> {
        self.state()
            .log
            .iter()
            .rev()
            .find_map(|(_, call)| match call {
                SharerCall::Pick { request } => Some(*request),
                _ => None,
            })
    }

    /// How many windows were given to the library and have not been released.
    pub(crate) fn live_windows(&self) -> usize {
        locked(&self.cells)
            .iter()
            .filter(|cell| {
                let progress = locked(&cell.progress);
                !progress.released && !progress.kept
            })
            .count()
    }

    /// How many windows were given to the library.
    pub(crate) fn windows_made(&self) -> usize {
        locked(&self.cells).len()
    }

    /// Whether the window stayed with the platform, because the library refused the answer that
    /// gave it: it is never called, and never released.
    pub(crate) fn is_kept(&self, window: u64) -> bool {
        let cells = locked(&self.cells);
        let cell = &cells[usize::try_from(window).unwrap() - 1];
        locked(&cell.progress).kept
    }

    /// The link that `start` of the window was given, 0 if it was not started.
    pub(crate) fn link(&self, window: u64) -> u64 {
        let cells = locked(&self.cells);
        let cell = &cells[usize::try_from(window).unwrap() - 1];
        locked(&cell.progress).link
    }

    // The answers of the platform to a pick.

    /// A window of the platform, as the pointer that an answer gives. The platform keeps it
    /// until the library releases it, or for ever if the library refuses the answer.
    pub(crate) fn new_window(self: &Arc<Self>) -> *mut c_void {
        let mut cells = locked(&self.cells);
        let cell = Arc::new(Cell {
            fake: Arc::downgrade(self),
            number: cells.len() as u64 + 1,
            progress: Mutex::new(Progress::default()),
            busy: AtomicBool::new(false),
        });
        cells.push(Arc::clone(&cell));
        Arc::as_ptr(&cell).cast_mut().cast()
    }

    /// A display of the platform that was given with `SCREEN_PRIVATE`, as the pointer that an
    /// answer gives.
    pub(crate) fn new_private_window(self: &Arc<Self>) -> *mut c_void {
        let window = self.new_window();
        // SAFETY: a pointer that `new_window` gave; the cells live as long as the fake.
        let cell = unsafe { &*window.cast_const().cast::<Cell>() };
        locked(&cell.progress).private = true;
        window
    }

    /// The platform keeps a window that the library did not take.
    pub(crate) fn keep(window: *mut c_void) {
        // SAFETY: a pointer that `new_window` gave; the cells live as long as the fake.
        let cell = unsafe { &*window.cast_const().cast::<Cell>() };
        locked(&cell.progress).kept = true;
    }

    /// Answers the pick of `request` with a window, as a platform does when the person chose one.
    /// A window that the library refused stays with the platform.
    pub(crate) fn choose(
        self: &Arc<Self>,
        registry: &Registry,
        request: u64,
        title: &[u8],
        app: &[u8],
        size: (u32, u32),
    ) -> Result<(), Error> {
        self.choose_kind(registry, request, PICKED_WINDOW, title, app, size)
    }

    /// Answers the pick of `request` with a whole display, as a platform does when the person
    /// chose a screen. It has no application.
    pub(crate) fn choose_display(
        self: &Arc<Self>,
        registry: &Registry,
        request: u64,
        title: &[u8],
        size: (u32, u32),
    ) -> Result<(), Error> {
        self.choose_kind(registry, request, PICKED_DISPLAY, title, b"", size)
    }

    fn choose_kind(
        self: &Arc<Self>,
        registry: &Registry,
        request: u64,
        kind: u32,
        title: &[u8],
        app: &[u8],
        size: (u32, u32),
    ) -> Result<(), Error> {
        let window = self.new_window();
        let picked = BatonPicked {
            size: size_of_u32::<BatonPicked>(),
            width: size.0,
            height: size.1,
            kind,
            context: window,
            title: BatonBytes::of(title),
            app: BatonBytes::of(app),
        };
        self.answer_raw(registry, request, Some(&picked), None)
            .inspect_err(|_| Self::keep(window))
    }

    /// Answers the pick of `request` with the sentence of a failure.
    pub(crate) fn fail_pick(
        &self,
        registry: &Registry,
        request: u64,
        sentence: &[u8],
    ) -> Result<(), Error> {
        self.answer_raw(registry, request, None, Some(&BatonBytes::of(sentence)))
    }

    /// Answers the pick of `request` with the news that the person gave up.
    pub(crate) fn give_up(&self, registry: &Registry, request: u64) -> Result<(), Error> {
        self.answer_raw(registry, request, None, None)
    }

    /// Answers the pick of `request` with these structs as they are, which may be wrong. The
    /// pick is over for the platform whatever the library makes of the answer.
    pub(crate) fn answer_raw(
        &self,
        registry: &Registry,
        request: u64,
        picked: Option<&BatonPicked>,
        failure: Option<&BatonBytes>,
    ) -> Result<(), Error> {
        let picked = picked.map_or(ptr::null(), ptr::from_ref);
        let failure = failure.map_or(ptr::null(), ptr::from_ref);
        // SAFETY: null, or structs that live through the call, whose bytes (if any) outlive it.
        let result = unsafe { registry.answer_pick(request, picked, failure) };
        self.state().waiting.remove(&request);
        result
    }
}

/// The fake behind a `context`.
///
/// # Safety
///
/// `context` is the pointer that [`FakeSharer::vtable`] gave, and the fake lives for the whole
/// test.
unsafe fn fake_of<'a>(context: *mut c_void) -> &'a FakeSharer {
    // SAFETY: as promised above.
    unsafe { &*context.cast_const().cast::<FakeSharer>() }
}

/// A strong reference to the fake behind a `context`.
///
/// # Safety
///
/// As for [`fake_of`].
unsafe fn arc_of(context: *mut c_void) -> Arc<FakeSharer> {
    let fake = context.cast_const().cast::<FakeSharer>();
    // SAFETY: the pointer is the one that `Arc::as_ptr` gave in `vtable`, of an `Arc` that is alive
    // for the whole test (`KEPT` holds it), so one more strong count can be taken and handed out.
    unsafe {
        Arc::increment_strong_count(fake);
        Arc::from_raw(fake)
    }
}

/// The window behind a pointer that an answer gave, and the fake it belongs to.
///
/// # Safety
///
/// `window` is a pointer that [`FakeSharer::new_window`] gave, and the fake lives for the whole
/// test.
unsafe fn window_of<'a>(window: *mut c_void) -> (&'a Cell, Arc<FakeSharer>) {
    assert!(!window.is_null(), "a call about a window with no pointer");
    // SAFETY: as promised above; the cells live as long as the fake.
    let cell = unsafe { &*window.cast_const().cast::<Cell>() };
    let fake = cell.fake.upgrade().expect("the fake outlives its windows");
    (cell, fake)
}

/// Marks a call about a window as under way, and complains if another is.
struct Busy<'a>(&'a Cell);

impl<'a> Busy<'a> {
    fn begin(cell: &'a Cell, owner: &FakeSharer, kind: &SharerCall) -> Self {
        if cell.busy.swap(true, Ordering::SeqCst) {
            owner.complain(format!(
                "{kind:?} while another call about window {} was under way",
                cell.number
            ));
        }
        Self(cell)
    }
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.busy.store(false, Ordering::SeqCst);
    }
}

/// Starts a call about a window: checks that the window may be called, and records the call.
fn about_window<'a>(cell: &'a Cell, owner: &FakeSharer, kind: &SharerCall) -> Busy<'a> {
    let busy = Busy::begin(cell, owner, kind);
    {
        let progress = locked(&cell.progress);
        if progress.kept {
            owner.complain(format!(
                "{kind:?} on window {}, which the library did not take",
                cell.number
            ));
        } else if progress.released {
            owner.complain(format!("{kind:?} after release, on window {}", cell.number));
        } else if progress.ended && *kind != SharerCall::Release {
            owner.complain(format!("{kind:?} after ended, on window {}", cell.number));
        }
    }
    owner.heard(cell.number, kind);
    busy
}

/// Checks the room for a sentence, as the header describes it.
fn check_message(fake: &FakeSharer, what: &str, message: BatonMessage) {
    if message.size != size_of_u32::<BatonMessage>()
        || message.cap != MESSAGE_MAX
        || message.len != 0
        || message.reserved != 0
        || message.text.is_null()
    {
        fake.complain(format!("{what}: the message is not empty: {message:?}"));
    }
}

/// Writes a failure the way a platform does: the text as far as there is room, and a length that
/// may claim more.
///
/// # Safety
///
/// `message` points to a struct of the interface that is the platform's to write, with room for
/// `cap` bytes at `text`.
unsafe fn write_failure(message: *mut BatonMessage, text: &[u8], claimed_len: u32) {
    // SAFETY: as promised above.
    unsafe {
        let cap = (&raw const (*message).cap).read() as usize;
        let written = text.len().min(cap);
        (&raw const (*message).text)
            .read()
            .copy_from_nonoverlapping(text.as_ptr(), written);
        (&raw mut (*message).len).write(claimed_len);
    }
}

unsafe extern "C" fn control(context: *mut c_void) -> u32 {
    // SAFETY: the context is the one `vtable` gave.
    let fake = unsafe { fake_of(context) };
    fake.heard(0, &SharerCall::Control);
    fake.state().control
}

unsafe extern "C" fn pick(context: *mut c_void, request: u64, message: *mut BatonMessage) -> i32 {
    // SAFETY: the context is the one `vtable` gave.
    let fake = unsafe { fake_of(context) };
    if message.is_null() {
        fake.complain("pick: a null message");
        return -1;
    }
    // SAFETY: not null, and the Rust side gives a struct of the interface.
    check_message(fake, "pick", unsafe { *message });
    let plan = {
        let mut state = fake.state();
        if request == 0 {
            state.complaints.push("pick: request 0".to_owned());
        }
        if !state.waiting.insert(request) {
            state
                .complaints
                .push(format!("pick: request {request} was given twice"));
        }
        state.pick.clone()
    };
    fake.heard(0, &SharerCall::Pick { request });
    match plan {
        PickPlan::Later => OK,
        PickPlan::Fail {
            code,
            text,
            claimed_len,
        } => {
            // The library does not take a pick down that failed.
            if code != OK {
                fake.state().waiting.remove(&request);
            }
            // SAFETY: `message` is not null and its struct is the platform's to write.
            unsafe { write_failure(message, &text, claimed_len) };
            code
        }
    }
}

unsafe extern "C" fn cancel_pick(context: *mut c_void, request: u64) {
    // SAFETY: the context is the one `vtable` gave.
    let fake = unsafe { fake_of(context) };
    if !fake.state().waiting.remove(&request) {
        fake.complain(format!(
            "cancel_pick: request {request} is not one that waits"
        ));
    }
    fake.heard(0, &SharerCall::CancelPick { request });
}

unsafe extern "C" fn screen(
    context: *mut c_void,
    flags: u32,
    picked: *mut BatonPicked,
    message: *mut BatonMessage,
) -> i32 {
    // SAFETY: the context is the one `vtable` gave.
    let fake = unsafe { fake_of(context) };
    if picked.is_null() || message.is_null() {
        fake.complain("screen: a null pointer");
        return -1;
    }
    // SAFETY: not null, and the Rust side gives structs of the interface.
    let (given, room) = unsafe { (*picked, *message) };
    check_message(fake, "screen", room);
    if given.size != size_of_u32::<BatonPicked>()
        || (given.width, given.height, given.kind) != (0, 0, 0)
        || !given.context.is_null()
        || given.title.len != 0
        || given.app.len != 0
    {
        fake.complain(format!("screen: the display is not empty: {given:?}"));
    }
    if flags & !SCREEN_PRIVATE != 0 {
        fake.complain(format!(
            "screen: a flag that the header does not have: {flags}"
        ));
    }
    fake.heard(0, &SharerCall::Screen { flags });
    let plan = {
        let mut state = fake.state();
        let plan = state.screen.clone();
        if matches!(plan, ScreenPlan::Waits(..)) {
            while !state.screen_open {
                state = fake
                    .screen_gate
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        }
        plan
    };
    let (kind, width, height, window) = match plan {
        ScreenPlan::Gives(width, height) | ScreenPlan::Waits(width, height) => {
            (PICKED_DISPLAY, width, height, true)
        }
        ScreenPlan::Writes {
            kind,
            width,
            height,
            window,
        } => (kind, width, height, window),
        ScreenPlan::Fails {
            code,
            text,
            claimed_len,
        } => {
            // SAFETY: `message` is not null and its struct is the platform's to write.
            unsafe { write_failure(message, &text, claimed_len) };
            return code;
        }
    };
    let window = if window {
        // SAFETY: the context is the one `vtable` gave.
        let fake = unsafe { arc_of(context) };
        if flags & SCREEN_PRIVATE != 0 {
            fake.new_private_window()
        } else {
            fake.new_window()
        }
    } else {
        ptr::null_mut()
    };
    // SAFETY: not null, and the struct is the platform's to write.
    unsafe {
        picked.write(BatonPicked {
            size: given.size,
            width,
            height,
            kind,
            context: window,
            title: BatonBytes::of(b""),
            app: BatonBytes::of(b""),
        });
    }
    OK
}

unsafe extern "C" fn prepare(
    picked: *mut c_void,
    takes: *const BatonTakes,
    format: *mut BatonFormat,
) -> i32 {
    // SAFETY: `picked` is a pointer that an answer gave.
    let (cell, fake) = unsafe { window_of(picked) };
    if takes.is_null() || format.is_null() {
        fake.complain("prepare: a null pointer");
        return -1;
    }
    // SAFETY: not null, and the Rust side gives structs of the interface.
    let (takes, format_in) = unsafe { (*takes, *format) };
    if takes.size != size_of_u32::<BatonTakes>() || takes.reserved != 0 {
        fake.complain(format!("prepare: the takes are not whole: {takes:?}"));
    }
    if format_in.size != size_of_u32::<BatonFormat>()
        || (
            format_in.codec,
            format_in.width,
            format_in.height,
            format_in.fps,
            format_in.reserved,
        ) != (0, 0, 0, 0, 0)
    {
        fake.complain(format!("prepare: the format is not empty: {format_in:?}"));
    }
    let _busy = about_window(
        cell,
        &fake,
        &SharerCall::Prepare {
            takes: (
                takes.max_width,
                takes.max_height,
                takes.max_fps,
                takes.codecs,
            ),
        },
    );
    {
        let mut progress = locked(&cell.progress);
        if progress.prepared {
            fake.complain(format!("prepare twice, on window {}", cell.number));
        }
        if progress.started {
            fake.complain(format!("prepare after start, on window {}", cell.number));
        }
        progress.prepared = true;
    }
    let plan = fake.state().prepare;
    let within = (
        CODEC_H264,
        takes.max_width.min(crate::fixture::WIDTH),
        takes.max_height.min(crate::fixture::HEIGHT),
        takes.max_fps.min(30),
    );
    let (chosen, code) = match plan {
        PreparePlan::Within => (within, OK),
        PreparePlan::Format(codec, width, height, fps) => ((codec, width, height, fps), OK),
        PreparePlan::Fail(code) => return code,
        PreparePlan::Answers(code) => (within, code),
    };
    // SAFETY: not null, and the struct is the platform's to write.
    unsafe {
        format.write(BatonFormat {
            size: format_in.size,
            codec: chosen.0,
            width: chosen.1,
            height: chosen.2,
            fps: chosen.3,
            reserved: 0,
        });
    }
    code
}

unsafe extern "C" fn start(picked: *mut c_void, link: u64, message: *mut BatonMessage) -> i32 {
    // SAFETY: `picked` is a pointer that an answer gave.
    let (cell, fake) = unsafe { window_of(picked) };
    if message.is_null() {
        fake.complain("start: a null message");
        return -1;
    }
    // SAFETY: not null, and the Rust side gives a struct of the interface.
    check_message(&fake, "start", unsafe { *message });
    if link == 0 {
        fake.complain("start: link 0");
    }
    let busy = about_window(cell, &fake, &SharerCall::Start { link });
    let plan = {
        let mut progress = locked(&cell.progress);
        if !progress.prepared {
            fake.complain(format!("start before prepare, on window {}", cell.number));
        }
        if progress.started || progress.start_failed {
            fake.complain(format!("start twice, on window {}", cell.number));
        }
        progress.link = link;
        fake.state().start.clone()
    };
    match plan {
        StartPlan::Works => {
            locked(&cell.progress).started = true;
            drop(busy);
            OK
        }
        StartPlan::Fail {
            code,
            text,
            claimed_len,
        } => {
            locked(&cell.progress).start_failed = true;
            // SAFETY: `message` is not null and its struct is the platform's to write.
            unsafe { write_failure(message, &text, claimed_len) };
            drop(busy);
            code
        }
    }
}

/// Checks that the window may be asked about its surroundings or told to post: it was started,
/// and the call is about input and not about a window that never got that far.
fn input_is_due(cell: &Cell, fake: &FakeSharer, what: &str) {
    let progress = locked(&cell.progress);
    if !progress.started {
        fake.complain(format!(
            "{what} before a start that worked, on window {}",
            cell.number
        ));
    }
}

unsafe extern "C" fn surroundings(picked: *mut c_void, around: *mut BatonSurroundings) -> i32 {
    // SAFETY: `picked` is a pointer that an answer gave.
    let (cell, fake) = unsafe { window_of(picked) };
    if around.is_null() {
        fake.complain("surroundings: a null struct");
        return -1;
    }
    // SAFETY: not null, and the Rust side gives a struct of the interface.
    let given = unsafe { *around };
    // What the library puts in before it asks is the careful answer, and the room for the windows
    // in front.
    if given.size != size_of_u32::<BatonSurroundings>()
        || given.locked != 1
        || given.on_screen != 0
        || given.keyboard != KEYBOARD_UNKNOWN
        || given.window != BatonRect::default()
        || given.fronts.is_null()
        || given.front_cap != FRONTS_MAX
        || given.front_count != 0
    {
        fake.complain(format!(
            "surroundings: the struct is not as agreed: {given:?}"
        ));
    }
    let _busy = about_window(cell, &fake, &SharerCall::Surroundings);
    input_is_due(cell, &fake, "surroundings");
    let plan = fake.state().around.clone();
    let (says, claimed) = match plan {
        AroundPlan::Fails(code) => return code,
        AroundPlan::WritesNothing => return OK,
        AroundPlan::Says(says) => {
            let count = u32::try_from(says.fronts.len()).unwrap();
            (says, count)
        }
        AroundPlan::Claims(says, claimed) => (says, claimed),
        AroundPlan::Numbers {
            locked,
            on_screen,
            keyboard,
        } => {
            // SAFETY: not null, and the struct is the platform's to write.
            unsafe {
                (*around).locked = locked;
                (*around).on_screen = on_screen;
                (*around).keyboard = keyboard;
                (*around).window = Around::default().window;
                (*around).front_count = 0;
            }
            return OK;
        }
    };
    // SAFETY: not null, and the struct is the platform's to write; the room for the windows in
    // front is `front_cap` long, and no more than that is written.
    unsafe {
        (*around).locked = u32::from(says.locked);
        (*around).on_screen = u32::from(says.on_screen);
        (*around).keyboard = says.keyboard;
        (*around).window = says.window;
        for (at, front) in says
            .fronts
            .iter()
            .take(given.front_cap as usize)
            .enumerate()
        {
            given.fronts.add(at).write(*front);
        }
        (*around).front_count = claimed;
    }
    OK
}

/// What a struct for `post` says, if it keeps the promises of the header, or the promise that it
/// breaks.
fn read_post(post: &BatonPost) -> Result<Post, String> {
    if post.size != size_of_u32::<BatonPost>() {
        return Err(format!("the size is {}", post.size));
    }
    let modifiers = u8::try_from(post.modifiers)
        .ok()
        .and_then(Modifiers::from_bits)
        .ok_or_else(|| format!("modifiers {:#x}", post.modifiers))?;
    match post.kind {
        POST_POINTER => {
            if (post.code, post.down, post.repeat, post.text.len) != (0, 0, 0, 0) {
                return Err("a pointer with the fields of a key".to_owned());
            }
            if !post.x.is_finite() || !post.y.is_finite() {
                return Err("a place that is not a number".to_owned());
            }
            let button = match post.button {
                BUTTON_LEFT => Some(Button::Left),
                BUTTON_RIGHT => Some(Button::Right),
                BUTTON_MIDDLE => Some(Button::Middle),
                _ => None,
            };
            // A move and a scroll have no button, which is a zero like the left one.
            let no_button = post.button == 0;
            let scrolls = (post.dx, post.dy) != (0.0, 0.0);
            let action = match (post.action, button) {
                (POINTER_MOVE, _) if no_button && !scrolls => PointerAction::Move,
                (POINTER_DOWN, Some(button)) if !scrolls => PointerAction::Down(button),
                (POINTER_UP, Some(button)) if !scrolls => PointerAction::Up(button),
                (POINTER_SCROLL, _) if no_button => PointerAction::Scroll {
                    dx: post.dx,
                    dy: post.dy,
                },
                _ => {
                    return Err(format!(
                        "action {}, button {}, scroll {} {}",
                        post.action, post.button, post.dx, post.dy
                    ));
                }
            };
            Ok(Post::Pointer {
                at: Point {
                    x: post.x,
                    y: post.y,
                },
                modifiers,
                action,
            })
        }
        POST_KEY => {
            if (post.x, post.y, post.dx, post.dy) != (0.0, 0.0, 0.0, 0.0)
                || (post.action, post.button) != (0, 0)
            {
                return Err("a key with the fields of a pointer".to_owned());
            }
            let code = u16::try_from(post.code)
                .ok()
                .filter(|code| *code <= MAX_KEY_CODE)
                .ok_or_else(|| format!("key code {}", post.code))?;
            if post.down > 1 || post.repeat > 1 {
                return Err(format!("down {}, repeat {}", post.down, post.repeat));
            }
            // 20 UTF-16 units are at most 60 bytes of UTF-8.
            let text = match post.text.len {
                0 => None,
                len if len <= 60 && !post.text.ptr.is_null() => {
                    // SAFETY: the library gives `len` bytes that live through the call.
                    let bytes = unsafe { slice::from_raw_parts(post.text.ptr, len) };
                    Some(
                        String::from_utf8(bytes.to_vec())
                            .map_err(|_| "text that is not UTF-8".to_owned())?,
                    )
                }
                len => return Err(format!("{len} bytes of text at {:?}", post.text.ptr)),
            };
            Ok(Post::Key {
                code,
                down: post.down == 1,
                modifiers,
                repeat: post.repeat == 1,
                text,
            })
        }
        kind => Err(format!("kind {kind}")),
    }
}

unsafe extern "C" fn post(picked: *mut c_void, post: *const BatonPost) -> i32 {
    // SAFETY: `picked` is a pointer that an answer gave.
    let (cell, fake) = unsafe { window_of(picked) };
    if post.is_null() {
        fake.complain("post: a null struct");
        return -1;
    }
    // SAFETY: not null, and the Rust side gives a struct of the interface.
    let read = read_post(unsafe { &*post });
    let post = read.unwrap_or_else(|why| {
        fake.complain(format!("post: {why}"));
        Post::Key {
            code: 0,
            down: false,
            modifiers: Modifiers::NONE,
            repeat: false,
            text: None,
        }
    });
    let _busy = about_window(cell, &fake, &SharerCall::Post(post));
    input_is_due(cell, &fake, "post");
    fake.state().post
}

unsafe extern "C" fn lift(picked: *mut c_void, lifted: u32) {
    // SAFETY: `picked` is a pointer that an answer gave.
    let (cell, fake) = unsafe { window_of(picked) };
    if lifted & !(LIFT_SCREEN | LIFT_INPUT) != 0 {
        fake.complain(format!(
            "lift: a bit that the header does not have: {lifted:#x}"
        ));
    }
    let _busy = about_window(cell, &fake, &SharerCall::Lift { lifted });
    input_is_due(cell, &fake, "lift");
    if !locked(&cell.progress).private {
        fake.complain(format!(
            "lift on window {}, which was not given as private",
            cell.number
        ));
    }
}

unsafe extern "C" fn hear(picked: *mut c_void, on: u32) {
    // SAFETY: `picked` is a pointer that an answer gave.
    let (cell, fake) = unsafe { window_of(picked) };
    if on > 1 {
        fake.complain(format!("hear: {on}"));
    }
    let _busy = about_window(cell, &fake, &SharerCall::Hear { on: on != 0 });
    input_is_due(cell, &fake, "hear");
}

unsafe extern "C" fn ended(picked: *mut c_void) {
    // SAFETY: `picked` is a pointer that an answer gave.
    let (cell, fake) = unsafe { window_of(picked) };
    let _busy = about_window(cell, &fake, &SharerCall::Ended);
    let mut progress = locked(&cell.progress);
    if !progress.started {
        fake.complain(format!(
            "ended on window {}, whose start did not work",
            cell.number
        ));
    }
    if progress.ended {
        fake.complain(format!("ended twice, on window {}", cell.number));
    }
    progress.ended = true;
}

unsafe extern "C" fn leave(picked: *mut c_void) {
    // SAFETY: `picked` is a pointer that an answer gave.
    let (cell, fake) = unsafe { window_of(picked) };
    let _busy = about_window(cell, &fake, &SharerCall::Leave);
    let mut progress = locked(&cell.progress);
    if !progress.started {
        fake.complain(format!(
            "leave on window {}, whose start did not work",
            cell.number
        ));
    }
    if !progress.private {
        fake.complain(format!(
            "leave on window {}, which was not given as private",
            cell.number
        ));
    }
    if progress.ended {
        fake.complain(format!("ended twice, on window {}", cell.number));
    }
    progress.ended = true;
}

unsafe extern "C" fn release(picked: *mut c_void) {
    // SAFETY: `picked` is a pointer that an answer gave.
    let (cell, fake) = unsafe { window_of(picked) };
    let _busy = about_window(cell, &fake, &SharerCall::Release);
    if std::mem::replace(&mut locked(&cell.progress).released, true) {
        fake.complain(format!("released twice, on window {}", cell.number));
    }
}

// The calls of the platform's capture into the library.

/// A picture as the encoder makes it, for [`sent`].
#[derive(Debug, Clone)]
pub(crate) struct Pic {
    pub(crate) flags: u32,
    pub(crate) capture_time: u64,
    pub(crate) length_size: u32,
    pub(crate) sample: Vec<u8>,
    pub(crate) sets: Vec<Vec<u8>>,
}

/// The microseconds since the session began, with a null pointer when `there` is false.
pub(crate) fn clock_to(registry: &Registry, link: u64, there: bool) -> Result<(), Error> {
    let mut micros = u64::MAX;
    let pointer = if there {
        &raw mut micros
    } else {
        ptr::null_mut()
    };
    // SAFETY: null, or a pointer to a `u64` that lives through the call.
    unsafe { registry.source_clock(link, pointer) }
}

/// The microseconds since the session began.
pub(crate) fn clock(registry: &Registry, link: u64) -> Result<u64, Error> {
    let mut micros = u64::MAX;
    // SAFETY: a pointer to a `u64` that lives through the call.
    unsafe { registry.source_clock(link, &raw mut micros) }?;
    Ok(micros)
}

/// Asks whether to make a picture, with a null pointer for the answer when `there` is false.
pub(crate) fn begin_to(registry: &Registry, link: u64, there: bool) -> Result<(), Error> {
    let mut kind = u32::MAX;
    let pointer = if there {
        &raw mut kind
    } else {
        ptr::null_mut()
    };
    // SAFETY: null, or a pointer to a `u32` that lives through the call.
    unsafe { registry.source_begin(link, pointer) }
}

/// Asks whether to make a picture, and which kind (a `BEGIN_` number).
pub(crate) fn begin(registry: &Registry, link: u64) -> Result<u32, Error> {
    let mut kind = u32::MAX;
    // SAFETY: a pointer to a `u32` that lives through the call.
    unsafe { registry.source_begin(link, &raw mut kind) }?;
    Ok(kind)
}

pub(crate) fn cancel(registry: &Registry, link: u64) -> Result<(), Error> {
    registry.source_cancel(link)
}

/// Asks whether a picture is wanted, with a null pointer for the answer when `there` is false.
pub(crate) fn recovery_pending_to(
    registry: &Registry,
    link: u64,
    there: bool,
) -> Result<(), Error> {
    let mut pending = u32::MAX;
    let pointer = if there {
        &raw mut pending
    } else {
        ptr::null_mut()
    };
    // SAFETY: null, or a pointer to a `u32` that lives through the call.
    unsafe { registry.source_recovery_pending(link, pointer) }
}

pub(crate) fn recovery_pending(registry: &Registry, link: u64) -> Result<bool, Error> {
    let mut pending = u32::MAX;
    // SAFETY: a pointer to a `u32` that lives through the call.
    unsafe { registry.source_recovery_pending(link, &raw mut pending) }?;
    Ok(pending != 0)
}

/// Hands over a picture, and gives what became of it: a `SENT_` number and the number of the
/// frame. The platform says nothing of the layout of the window, as one that cannot post does.
pub(crate) fn sent(registry: &Registry, link: u64, pic: &Pic) -> Result<(u32, u64), Error> {
    sent_with(registry, link, pic, BatonGeometry::default())
}

/// As [`sent`], with the layout of the window in the picture.
pub(crate) fn sent_with(
    registry: &Registry,
    link: u64,
    pic: &Pic,
    geometry: BatonGeometry,
) -> Result<(u32, u64), Error> {
    sent_sized(registry, link, pic, geometry, size_of_u32::<BatonPicture>())
}

/// As [`sent_with`], with the struct saying that it is `size` bytes long, as a platform does that
/// was written when the struct was shorter: what lies beyond `size` is in the memory, and is not
/// to be read.
pub(crate) fn sent_sized(
    registry: &Registry,
    link: u64,
    pic: &Pic,
    geometry: BatonGeometry,
    size: u32,
) -> Result<(u32, u64), Error> {
    let sets: Vec<BatonBytes> = pic.sets.iter().map(|set| BatonBytes::of(set)).collect();
    let picture = BatonPicture {
        size,
        flags: pic.flags,
        capture_time: pic.capture_time,
        length_size: pic.length_size,
        reserved: 0,
        sample: BatonBytes::of(&pic.sample),
        sets: if sets.is_empty() {
            ptr::null()
        } else {
            sets.as_ptr()
        },
        set_count: sets.len(),
        geometry,
    };
    let (mut outcome, mut number) = (u32::MAX, u64::MAX);
    // SAFETY: a struct of the interface that is whole, with bytes that outlive the call, and
    // pointers to a `u32` and a `u64` that live through it.
    unsafe { registry.source_sent(link, &raw const picture, &raw mut outcome, &raw mut number) }?;
    Ok((outcome, number))
}

/// Hands over a picture as the struct stands, which may be wrong, with the pointers for what
/// became of it that are given (`None` is a null pointer).
///
/// # Safety
///
/// The memory that `picture` describes can be read during the call if the library is to read
/// it: the library reads no more of it than the header allows.
pub(crate) unsafe fn sent_raw(
    registry: &Registry,
    link: u64,
    picture: Option<&BatonPicture>,
    outcome: Option<&mut u32>,
    number: Option<&mut u64>,
) -> Result<(), Error> {
    let picture = picture.map_or(ptr::null(), ptr::from_ref);
    let outcome = outcome.map_or(ptr::null_mut(), ptr::from_mut);
    let number = number.map_or(ptr::null_mut(), ptr::from_mut);
    // SAFETY: the caller's promise about the picture is this function's own, and the others are
    // null or point to values that live through the call.
    unsafe { registry.source_sent(link, picture, outcome, number) }
}

/// The form of sound that an AAC encoder makes: this many samples a second, in this many channels,
/// with this many samples of each in a packet.
pub(crate) fn aac(rate: u32, channels: u32, samples: u32) -> BatonSound {
    BatonSound {
        size: size_of_u32::<BatonSound>(),
        codec: SOUND_CODEC_AAC,
        rate,
        channels,
        samples,
        reserved: 0,
    }
}

/// Says what the packets of sound are.
pub(crate) fn sound_begin(registry: &Registry, link: u64, sound: &BatonSound) -> Result<(), Error> {
    // SAFETY: a struct of the interface that is whole.
    unsafe { registry.source_sound_begin(link, sound) }
}

/// Says what the packets of sound are, with the struct as given: `None` is a null pointer.
///
/// # Safety
///
/// `sound` is a struct that lives through the call and is at least as long as its size says, or
/// `None`.
pub(crate) unsafe fn sound_begin_raw(
    registry: &Registry,
    link: u64,
    sound: Option<&BatonSound>,
) -> Result<(), Error> {
    let pointer = sound.map_or(ptr::null(), ptr::from_ref);
    // SAFETY: null, or a struct of the interface that lives through the call, as promised.
    unsafe { registry.source_sound_begin(link, pointer) }
}

/// Says that the sound that is wanted cannot be sent, for a `SILENT_` reason.
pub(crate) fn sound_silent(registry: &Registry, link: u64, reason: u32) -> Result<(), Error> {
    registry.source_sound_silent(link, reason)
}

/// Hands over a packet of sound, and gives what became of it: a `SOUND_` number and the number of
/// the packet.
pub(crate) fn sound(
    registry: &Registry,
    link: u64,
    capture_time: u64,
    bytes: &[u8],
) -> Result<(u32, u64), Error> {
    let packet = BatonSoundPacket {
        size: size_of_u32::<BatonSoundPacket>(),
        reserved: 0,
        capture_time,
        bytes: BatonBytes::of(bytes),
    };
    let (mut outcome, mut number) = (u32::MAX, u64::MAX);
    // SAFETY: a struct of the interface that is whole, with bytes that outlive the call, and
    // pointers to a `u32` and a `u64` that live through it.
    unsafe { registry.source_sound(link, &raw const packet, &raw mut outcome, &raw mut number) }?;
    Ok((outcome, number))
}

/// Hands over a packet of sound as the struct stands, which may be wrong, with the pointers for
/// what became of it that are given (`None` is a null pointer).
///
/// # Safety
///
/// The memory that `packet` describes can be read during the call if the library is to read it:
/// the library reads no more of it than the header allows.
pub(crate) unsafe fn sound_raw(
    registry: &Registry,
    link: u64,
    packet: Option<&BatonSoundPacket>,
    outcome: Option<&mut u32>,
    number: Option<&mut u64>,
) -> Result<(), Error> {
    let packet = packet.map_or(ptr::null(), ptr::from_ref);
    let outcome = outcome.map_or(ptr::null_mut(), ptr::from_mut);
    let number = number.map_or(ptr::null_mut(), ptr::from_mut);
    // SAFETY: the caller's promise about the packet is this function's own, and the others are
    // null or point to values that live through the call.
    unsafe { registry.source_sound(link, packet, outcome, number) }
}

/// The window changed at `capture_time`.
pub(crate) fn changed(registry: &Registry, link: u64, capture_time: u64) -> Result<(), Error> {
    registry.source_changed(link, capture_time)
}

/// The size of the picture changed.
pub(crate) fn new_epoch(
    registry: &Registry,
    link: u64,
    (codec, width, height, fps): (u32, u32, u32, u32),
) -> Result<(), Error> {
    let format = BatonFormat {
        size: size_of_u32::<BatonFormat>(),
        codec,
        width,
        height,
        fps,
        reserved: 0,
    };
    // SAFETY: a struct of the interface that is whole.
    unsafe { registry.source_new_epoch(link, &raw const format) }
}

/// The size of the picture changed, with the struct as given: `None` is a null pointer.
///
/// # Safety
///
/// `format` is a struct that lives through the call and is at least as long as its size says,
/// or `None`.
pub(crate) unsafe fn new_epoch_raw(
    registry: &Registry,
    link: u64,
    format: Option<&BatonFormat>,
) -> Result<(), Error> {
    let pointer = format.map_or(ptr::null(), ptr::from_ref);
    // SAFETY: null, or a struct of the interface that lives through the call, as promised.
    unsafe { registry.source_new_epoch(link, pointer) }
}

pub(crate) fn pause(registry: &Registry, link: u64, reason: u32) -> Result<(), Error> {
    registry.source_pause(link, reason)
}

pub(crate) fn resume(registry: &Registry, link: u64) -> Result<(), Error> {
    registry.source_resume(link)
}

/// Ends the session. `None` is a `NULL` pointer for the sentence.
pub(crate) fn end(registry: &Registry, link: u64, sentence: Option<&[u8]>) -> Result<(), Error> {
    let bytes = sentence.map(BatonBytes::of);
    let pointer = bytes.as_ref().map_or(ptr::null(), ptr::from_ref);
    // SAFETY: null, or a `BatonBytes` with bytes that outlive the call.
    unsafe { registry.source_end(link, pointer) }
}
