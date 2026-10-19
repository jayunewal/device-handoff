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
//! The other half of a platform, the capture thread that asks `begin` and hands over what the
//! encoder made, is a set of functions that call the library the way Swift would, with the same
//! structs and pointers: [`begin`], [`sent`] and the others.

#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    collections::BTreeSet,
    ffi::c_void,
    ptr,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::{
    Error, Registry,
    abi::{
        ABI_VERSION, BatonBytes, BatonFormat, BatonMessage, BatonPicked, BatonPicture,
        BatonSharerVTable, BatonTakes, CODEC_H264, MESSAGE_MAX, OK, size_of_u32,
    },
    platform::SharerCalls,
};

/// What the platform got, as a test can compare it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SharerCall {
    Control,
    Pick {
        request: u64,
    },
    CancelPick {
        request: u64,
    },
    /// What the other device takes: the widest and the highest picture, the rate and the codecs.
    Prepare {
        takes: (u32, u32, u32, u32),
    },
    Start {
        link: u64,
    },
    Ended,
    Release,
}

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

/// Called inside a call of the platform, with the call and the number of the window it is about
/// (0 for the calls about the device), to make the platform call the library.
pub(crate) type Hook = Arc<dyn Fn(u64, &SharerCall) + Send + Sync>;

struct State {
    log: Vec<(u64, SharerCall)>,
    complaints: Vec<String>,
    control: u32,
    pick: PickPlan,
    prepare: PreparePlan,
    start: StartPlan,
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
            state: Mutex::new(State {
                log: Vec::new(),
                complaints: Vec::new(),
                control: 0,
                pick: PickPlan::Later,
                prepare: PreparePlan::Within,
                start: StartPlan::Works,
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
        }
    }

    /// Registers this platform in `registry`.
    pub(crate) fn register_in(self: &Arc<Self>, registry: &Registry) -> Result<(), Error> {
        register_table(registry, &self.vtable())
    }

    /// The calls that registration makes of this platform.
    pub(crate) fn platform(self: &Arc<Self>) -> SharerCalls {
        read_table(&self.vtable()).expect("the vtable of the fake is whole")
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

    pub(crate) fn plan_pick(&self, plan: PickPlan) {
        self.state().pick = plan;
    }

    pub(crate) fn plan_prepare(&self, plan: PreparePlan) {
        self.state().prepare = plan;
    }

    pub(crate) fn plan_start(&self, plan: StartPlan) {
        self.state().start = plan;
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
        let window = self.new_window();
        let picked = BatonPicked {
            size: size_of_u32::<BatonPicked>(),
            width: size.0,
            height: size.1,
            reserved: 0,
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
/// frame.
pub(crate) fn sent(registry: &Registry, link: u64, pic: &Pic) -> Result<(u32, u64), Error> {
    let sets: Vec<BatonBytes> = pic.sets.iter().map(|set| BatonBytes::of(set)).collect();
    let picture = BatonPicture {
        size: size_of_u32::<BatonPicture>(),
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
