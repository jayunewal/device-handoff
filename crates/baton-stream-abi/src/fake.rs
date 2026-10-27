// SPDX-License-Identifier: Apache-2.0

//! A platform that the tests play, through the same C functions that a real one provides.
//!
//! It logs every call it gets, checks each one against what the header promises (sizes, pointers,
//! that a screen is called only between `open` and `release`, that nothing follows `ended` but
//! `release`, that `release` comes once) and writes down a complaint for each promise that is
//! broken. A test holds the Rust side to the header with [`Fake::complaints`], and makes the
//! platform fail, lie or call back into the library with the settings.
//!
//! A platform that unlocks the screens of other devices with a face also gets `unlock_key`, which
//! gives the key that the test chose ([`KeyPlan`]), and `unlock_open` and `unlock_cancel`, which
//! are logged ([`Call::UnlockOpen`], [`Call::UnlockCancel`]). The answer of the person, which a
//! real platform gives from its own thread when the face is recognised or not, is
//! [`unlock_opened`] and [`unlock_declined`].

#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    ffi::c_void,
    mem::offset_of,
    slice,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError},
    },
    thread,
    time::Duration,
};

use crate::{
    Error, Registry,
    abi::{
        ABI_VERSION, BatonBytes, BatonDecoderState, BatonFrame, BatonLimits, BatonMessage,
        BatonPlay, BatonScreen, BatonSound, BatonSourceMessage, BatonUnlockKey,
        BatonUnlockQuestion, BatonUnlockSource, BatonViewerVTable, BatonWindow, CODEC_H264,
        DEVICE_ID_BYTES, MESSAGE_MAX, OK, PLAY_FIRST, SEALING_ECIES_P256, UNLOCK_KEY_MAX,
        UNLOCK_SEALED_MAX, size_of_u32,
    },
    platform::Calls,
};

/// A frame as the platform got it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Decoded {
    pub(crate) flags: u32,
    pub(crate) number: u64,
    pub(crate) depends_on: u64,
    pub(crate) capture_time: u64,
    pub(crate) chain: u32,
    pub(crate) epoch: u32,
    pub(crate) sample: Vec<u8>,
    pub(crate) sets: Vec<Vec<u8>>,
    pub(crate) sequence_sets: usize,
}

/// A packet of sound as the platform got it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Played {
    pub(crate) flags: u32,
    pub(crate) number: u64,
    pub(crate) capture_time: u64,
    pub(crate) lost_before: u64,
    pub(crate) packet: Vec<u8>,
}

/// One call that the platform got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Call {
    Limits,
    Open {
        title: String,
        app: String,
        width: u32,
        height: u32,
        control: u32,
        link: u64,
    },
    Decode(Decoded),
    Show(u64),
    Source(BatonSourceMessage),
    State,
    Ended,
    Release,
    /// The platform was told what the packets of sound are.
    Sound(BatonSound),
    /// The platform was handed a packet of sound to decode and play.
    Play(Played),
    /// The run of sound is over, for this `SILENT_` reason.
    Stopped(u32),
    /// The platform was asked for the key that an unlock of this device is sealed to.
    UnlockKey {
        id: Vec<u8>,
        name: String,
        renew: bool,
    },
    /// The platform was given a question to answer with the face of the person.
    UnlockOpen {
        number: u64,
        sealed: Vec<u8>,
        id: Vec<u8>,
        name: String,
    },
    /// The platform was told that the question it holds is not open any more.
    UnlockCancel,
}

/// What `open` does.
#[derive(Debug, Clone)]
pub(crate) enum OpenPlan {
    /// Makes a screen that the user interface draws with the texture `id`.
    Screen { id: i64 },
    /// Fails. It writes `text` (as far as there is room) and says that it wrote `claimed_len`
    /// bytes, which a platform that is wrong about it can make more than there was room for.
    Fail { text: Vec<u8>, claimed_len: u32 },
    /// Says that it worked and gives no pointer for the screen.
    NoPointer,
}

/// What `unlock_key` does.
#[derive(Debug, Clone)]
pub(crate) enum KeyPlan {
    /// Gives this key, of a `SEALING_` number.
    Gives { sealing: u32, key: Vec<u8> },
    /// Returns this code (a `KEY_` number, or another) and gives no key. It writes `text` (as far
    /// as there is room) and says that it wrote `claimed_len` bytes, as `OpenPlan::Fail` does.
    Fails {
        code: i32,
        text: Vec<u8>,
        claimed_len: u32,
    },
    /// Returns `OK` and says that the key is `len` bytes of this `SEALING_` number, whatever the
    /// room holds, which a platform that is wrong about the header may do: also beyond the room.
    Claims { sealing: u32, len: u32 },
}

/// A public key of P-256 in the form that the library takes: `0x04` and the two coordinates, here
/// counted from 1.
pub(crate) fn a_public_key() -> Vec<u8> {
    let mut key = vec![0x04];
    key.extend(1..=64_u8);
    key
}

/// Called inside a call of the platform, with the call, to make the platform call the library.
pub(crate) type Hook = Arc<dyn Fn(&Call) + Send + Sync>;

/// Stands behind a call of the library that would wait for ever if a guard of the library were
/// missing, such as the refusal of a call from inside a call. When the call has not returned
/// after half a minute the process ends with a message, so that a guard that is missing fails a
/// run at once and does not hold it up until something else gives up. It stops when it is dropped.
pub(crate) struct Watchdog {
    _done: mpsc::Sender<()>,
}

impl Watchdog {
    pub(crate) fn start(what: &'static str) -> Self {
        let (done, wait) = mpsc::channel();
        thread::spawn(move || {
            if wait.recv_timeout(Duration::from_secs(30)) == Err(RecvTimeoutError::Timeout) {
                eprintln!("{what} did not return: it is waiting for the library");
                std::process::abort();
            }
        });
        Self { _done: done }
    }
}

struct State {
    log: Vec<(u64, Call)>,
    complaints: Vec<String>,
    limits: BatonLimits,
    limits_code: i32,
    open: OpenPlan,
    decode_code: i32,
    decode_us: u32,
    thermal: u32,
    /// Whether the platform registers as one that plays sound.
    sound: bool,
    /// Whether the platform registers as one that unlocks with a face.
    face: bool,
    key: KeyPlan,
    /// What `unlock_open` returns: `OK` when the platform took the question.
    open_code: i32,
    hook: Option<Hook>,
}

/// One screen of the fake: what the pointer that `open` gave stands for.
struct Cell {
    fake: Weak<Fake>,
    number: u64,
    released: AtomicBool,
    ended: AtomicBool,
}

/// The platform of a test.
pub(crate) struct Fake {
    state: Mutex<State>,
    /// The screens that were made. They stay until the fake is dropped, so a call that comes
    /// after `release` finds the flag and not freed memory.
    cells: Mutex<Vec<Arc<Cell>>>,
    me: Weak<Self>,
}

/// Registers `table` in `registry` as a platform does.
pub(crate) fn register_table(registry: &Registry, table: &BatonViewerVTable) -> Result<(), Error> {
    // SAFETY: `table` is a struct of the interface that is whole and that nothing writes to
    // meanwhile; its context is the fake's, which lives for the whole test.
    unsafe { registry.register_viewer(table) }
}

/// The calls that registration makes of `table`.
pub(crate) fn read_table(table: &BatonViewerVTable) -> Result<Calls, Error> {
    // SAFETY: as above.
    unsafe { Calls::read(table) }
}

/// Every fake that was made, so that none is dropped while a screen of it is still alive.
static KEPT: Mutex<Vec<Arc<Fake>>> = Mutex::new(Vec::new());

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Fake {
    /// A platform that can show 1920 by 1200 pixels at 60 frames a second in H.264, and makes a
    /// screen when it is asked to.
    ///
    /// It lives for as long as the process does: a vtable holds its address and not a reference,
    /// and a session that is still ending when a test returns must find it where it was.
    pub(crate) fn new() -> Arc<Self> {
        let fake = Arc::new_cyclic(|me| Self {
            state: Mutex::new(State {
                log: Vec::new(),
                complaints: Vec::new(),
                limits: Self::default_limits(),
                limits_code: OK,
                open: OpenPlan::Screen { id: 100 },
                decode_code: OK,
                decode_us: 1000,
                thermal: 0,
                sound: true,
                face: true,
                key: KeyPlan::Gives {
                    sealing: SEALING_ECIES_P256,
                    key: a_public_key(),
                },
                open_code: OK,
                hook: None,
            }),
            cells: Mutex::new(Vec::new()),
            me: me.clone(),
        });
        locked(&KEPT).push(Arc::clone(&fake));
        fake
    }

    /// What a new fake says it can show.
    pub(crate) fn default_limits() -> BatonLimits {
        BatonLimits {
            size: size_of_u32::<BatonLimits>(),
            max_width: 1920,
            max_height: 1200,
            max_fps: 60,
            codecs: CODEC_H264,
            reserved: 0,
        }
    }

    /// The calls that registration makes of this platform.
    pub(crate) fn platform(self: &Arc<Self>) -> Calls {
        read_table(&self.registered_table()).expect("the vtable of the fake is whole")
    }

    /// Registers this platform in `registry`: with the whole table, or with the one of an older
    /// platform if a test said that this one is ([`Self::plays_no_sound`], [`Self::has_no_face`]).
    pub(crate) fn register_in(self: &Arc<Self>, registry: &Registry) -> Result<(), Error> {
        register_table(registry, &self.registered_table())
    }

    /// The table that registers this platform: the whole one, or the one of an older platform.
    fn registered_table(self: &Arc<Self>) -> BatonViewerVTable {
        if !self.state().sound {
            self.vtable_without_sound()
        } else if !self.state().face {
            self.vtable_without_face()
        } else {
            self.vtable()
        }
    }

    /// The vtable that registers this platform.
    pub(crate) fn vtable(self: &Arc<Self>) -> BatonViewerVTable {
        BatonViewerVTable {
            version: ABI_VERSION,
            size: size_of_u32::<BatonViewerVTable>(),
            context: Arc::as_ptr(self).cast_mut().cast(),
            limits: Some(limits),
            open: Some(open),
            decode: Some(decode),
            show: Some(show),
            source: Some(source),
            state: Some(state),
            ended: Some(ended),
            release: Some(release),
            sound: Some(sound),
            play: Some(play),
            stopped: Some(stopped),
            unlock_key: Some(unlock_key),
            unlock_open: Some(unlock_open),
            unlock_cancel: Some(unlock_cancel),
        }
    }

    /// The vtable of a platform that was written before sound was carried: it ends at `release`,
    /// and the library reads the three functions that it lacks as missing.
    pub(crate) fn vtable_without_sound(self: &Arc<Self>) -> BatonViewerVTable {
        BatonViewerVTable {
            size: u32::try_from(offset_of!(BatonViewerVTable, sound)).unwrap(),
            sound: None,
            play: None,
            stopped: None,
            unlock_key: None,
            unlock_open: None,
            unlock_cancel: None,
            ..self.vtable()
        }
    }

    /// The vtable of a platform that was written before a face could unlock: it ends at
    /// `stopped`, and the library reads the three functions that it lacks as missing.
    pub(crate) fn vtable_without_face(self: &Arc<Self>) -> BatonViewerVTable {
        BatonViewerVTable {
            size: u32::try_from(offset_of!(BatonViewerVTable, unlock_key)).unwrap(),
            unlock_key: None,
            unlock_open: None,
            unlock_cancel: None,
            ..self.vtable()
        }
    }

    /// The vtable of a platform that gives some of the three functions of the face and not all:
    /// it keeps its size, and the library reads all three as missing.
    pub(crate) fn vtable_with_some_face(self: &Arc<Self>) -> BatonViewerVTable {
        BatonViewerVTable {
            unlock_cancel: None,
            ..self.vtable()
        }
    }

    /// Registers this platform in `registry`, as one that plays no sound.
    pub(crate) fn register_without_sound_in(
        self: &Arc<Self>,
        registry: &Registry,
    ) -> Result<(), Error> {
        register_table(registry, &self.vtable_without_sound())
    }

    fn state(&self) -> MutexGuard<'_, State> {
        locked(&self.state)
    }

    /// Records a call, and runs the hook, if there is one, with nothing locked.
    fn heard(&self, screen: u64, call: &Call) {
        let hook = {
            let mut state = self.state();
            state.log.push((screen, call.clone()));
            state.hook.clone()
        };
        if let Some(hook) = hook {
            hook(call);
        }
    }

    fn complain(&self, what: impl Into<String>) {
        self.state().complaints.push(what.into());
    }

    // What the tests set.

    pub(crate) fn set_limits(&self, set: impl FnOnce(&mut BatonLimits)) {
        set(&mut self.state().limits);
    }

    pub(crate) fn fail_limits(&self, code: i32) {
        self.state().limits_code = code;
    }

    /// The platform registers as one that was written before sound was carried.
    pub(crate) fn plays_no_sound(&self) {
        self.state().sound = false;
    }

    /// The platform registers as one that was written before a face could unlock.
    pub(crate) fn has_no_face(&self) {
        self.state().face = false;
    }

    /// What `unlock_key` does from now on.
    pub(crate) fn plan_key(&self, plan: KeyPlan) {
        self.state().key = plan;
    }

    /// What `unlock_open` returns from now on: `OK` when the platform takes the question.
    pub(crate) fn plan_open_question(&self, code: i32) {
        self.state().open_code = code;
    }

    pub(crate) fn plan_open(&self, plan: OpenPlan) {
        self.state().open = plan;
    }

    pub(crate) fn fail_decode(&self, code: i32) {
        self.state().decode_code = code;
    }

    pub(crate) fn set_state(&self, decode_us: u32, thermal: u32) {
        let mut state = self.state();
        state.decode_us = decode_us;
        state.thermal = thermal;
    }

    pub(crate) fn on_call(&self, hook: impl Fn(&Call) + Send + Sync + 'static) {
        self.state().hook = Some(Arc::new(hook));
    }

    // What the tests read.

    /// Every call, in order, with the number of the screen it was about (0 for the calls about
    /// the device: the first screen is 1).
    pub(crate) fn log(&self) -> Vec<(u64, Call)> {
        self.state().log.clone()
    }

    /// The calls about one screen, in order.
    pub(crate) fn calls(&self, screen: u64) -> Vec<Call> {
        self.state()
            .log
            .iter()
            .filter(|(of, _)| *of == screen)
            .map(|(_, call)| call.clone())
            .collect()
    }

    /// How many calls of this kind there were, over all screens.
    pub(crate) fn count(&self, kind: impl Fn(&Call) -> bool) -> usize {
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

    /// How many screens were made and not released.
    pub(crate) fn live_screens(&self) -> usize {
        locked(&self.cells)
            .iter()
            .filter(|cell| !cell.released.load(Ordering::SeqCst))
            .count()
    }

    /// How many screens were made.
    pub(crate) fn screens_made(&self) -> usize {
        locked(&self.cells).len()
    }
}

/// The fake behind a `context`.
///
/// # Safety
///
/// `context` is the pointer that [`Fake::vtable`] gave, and the fake lives for the whole test.
unsafe fn fake_of<'a>(context: *mut c_void) -> &'a Fake {
    // SAFETY: as promised above.
    unsafe { &*context.cast_const().cast::<Fake>() }
}

/// The screen behind a pointer that `open` gave, and the fake it belongs to.
///
/// # Safety
///
/// `screen` is a pointer that `open` gave, and the fake lives for the whole test.
unsafe fn screen_of<'a>(screen: *mut c_void) -> (&'a Cell, Arc<Fake>) {
    assert!(!screen.is_null(), "a call about a screen with no pointer");
    // SAFETY: as promised above; the cells live as long as the fake.
    let cell = unsafe { &*screen.cast_const().cast::<Cell>() };
    let fake = cell.fake.upgrade().expect("the fake outlives its screens");
    (cell, fake)
}

/// Starts a call about a screen: checks that the screen may be called, and records the call.
fn about_screen(screen: &Cell, fake: &Fake, call: &Call) {
    if screen.released.load(Ordering::SeqCst) {
        fake.complain(format!(
            "{call:?} after release, on screen {}",
            screen.number
        ));
    } else if screen.ended.load(Ordering::SeqCst) && *call != Call::Release {
        fake.complain(format!("{call:?} after ended, on screen {}", screen.number));
    }
    fake.heard(screen.number, call);
}

fn bytes(of: &BatonBytes) -> Vec<u8> {
    if of.len == 0 {
        return Vec::new();
    }
    assert!(!of.ptr.is_null(), "bytes with a length and no pointer");
    // SAFETY: the Rust side promises `len` bytes at `ptr` for the call, and the assert above
    // checked the pointer.
    unsafe { slice::from_raw_parts(of.ptr, of.len) }.to_vec()
}

unsafe extern "C" fn limits(context: *mut c_void, out: *mut BatonLimits) -> i32 {
    // SAFETY: the context is the one `vtable` gave.
    let fake = unsafe { fake_of(context) };
    fake.heard(0, &Call::Limits);
    if out.is_null() {
        fake.complain("limits: a null struct");
        return -1;
    }
    // SAFETY: not null, and the Rust side gives a struct of the interface.
    let size = unsafe { (&raw const (*out).size).read() };
    if size != size_of_u32::<BatonLimits>() {
        fake.complain(format!("limits: size {size}"));
    }
    let (set, code) = {
        let state = fake.state();
        (state.limits, state.limits_code)
    };
    if code == OK {
        // SAFETY: not null, and the struct is the platform's to write.
        unsafe { out.write(BatonLimits { size, ..set }) };
    }
    code
}

unsafe extern "C" fn open(
    context: *mut c_void,
    window: *const BatonWindow,
    link: u64,
    screen: *mut BatonScreen,
    message: *mut BatonMessage,
) -> i32 {
    // SAFETY: the context is the one `vtable` gave.
    let fake = unsafe { fake_of(context) };
    if window.is_null() || screen.is_null() || message.is_null() {
        fake.complain("open: a null pointer");
        return -1;
    }
    // SAFETY: not null, and the Rust side gives structs of the interface.
    let (window, screen_in, message_in) = unsafe { (*window, *screen, *message) };
    if window.size != size_of_u32::<BatonWindow>() {
        fake.complain(format!("open: window size {}", window.size));
    }
    if screen_in.size != size_of_u32::<BatonScreen>()
        || screen_in.reserved != 0
        || !screen_in.context.is_null()
        || screen_in.id != 0
    {
        fake.complain(format!("open: the screen is not empty: {screen_in:?}"));
    }
    if message_in.size != size_of_u32::<BatonMessage>()
        || message_in.cap != MESSAGE_MAX
        || message_in.len != 0
        || message_in.reserved != 0
        || message_in.text.is_null()
    {
        fake.complain(format!("open: the message is not empty: {message_in:?}"));
    }
    if link == 0 {
        fake.complain("open: link 0");
    }
    fake.heard(
        0,
        &Call::Open {
            title: String::from_utf8(bytes(&window.title)).expect("the title is UTF-8"),
            app: String::from_utf8(bytes(&window.app)).expect("the application is UTF-8"),
            width: window.width,
            height: window.height,
            control: window.control,
            link,
        },
    );
    let plan = fake.state().open.clone();
    match plan {
        OpenPlan::Screen { id } => {
            let cell = {
                let mut cells = locked(&fake.cells);
                let cell = Arc::new(Cell {
                    fake: fake.me.clone(),
                    number: cells.len() as u64 + 1,
                    released: AtomicBool::new(false),
                    ended: AtomicBool::new(false),
                });
                cells.push(Arc::clone(&cell));
                cell
            };
            // SAFETY: `screen` is not null and its struct is the platform's to write.
            unsafe {
                (&raw mut (*screen).context).write(Arc::as_ptr(&cell).cast_mut().cast());
                (&raw mut (*screen).id).write(id);
            }
            OK
        }
        OpenPlan::NoPointer => OK,
        OpenPlan::Fail { text, claimed_len } => {
            let written = text.len().min(message_in.cap as usize);
            // SAFETY: `text` points to `cap` bytes that are the platform's to write, and so is
            // the struct.
            unsafe {
                message_in
                    .text
                    .copy_from_nonoverlapping(text.as_ptr(), written);
                (&raw mut (*message).len).write(claimed_len);
            }
            -2
        }
    }
}

unsafe extern "C" fn decode(screen: *mut c_void, frame: *const BatonFrame) -> i32 {
    // SAFETY: `screen` is a pointer that `open` gave.
    let (cell, fake) = unsafe { screen_of(screen) };
    if frame.is_null() {
        fake.complain("decode: a null frame");
        return -1;
    }
    // SAFETY: not null, and the Rust side gives a struct of the interface.
    let frame = unsafe { *frame };
    let sets: Vec<Vec<u8>> = if frame.set_count == 0 {
        Vec::new()
    } else {
        assert!(!frame.sets.is_null(), "sets are missing");
        // SAFETY: `set_count` entries at `sets` are valid for the call, as the header says.
        unsafe { slice::from_raw_parts(frame.sets, frame.set_count) }
            .iter()
            .map(bytes)
            .collect()
    };
    if frame.size != size_of_u32::<BatonFrame>() {
        fake.complain(format!("decode: size {}", frame.size));
    }
    if frame.sequence_sets > frame.set_count {
        fake.complain("decode: more sequence sets than sets");
    }
    if frame.set_count == 0 && !frame.sets.is_null() {
        fake.complain("decode: a pointer for no sets");
    }
    about_screen(
        cell,
        &fake,
        &Call::Decode(Decoded {
            flags: frame.flags,
            number: frame.number,
            depends_on: frame.depends_on,
            capture_time: frame.capture_time,
            chain: frame.chain,
            epoch: frame.epoch,
            sample: bytes(&frame.sample),
            sets,
            sequence_sets: frame.sequence_sets,
        }),
    );
    fake.state().decode_code
}

unsafe extern "C" fn show(screen: *mut c_void, number: u64) {
    // SAFETY: `screen` is a pointer that `open` gave.
    let (cell, fake) = unsafe { screen_of(screen) };
    about_screen(cell, &fake, &Call::Show(number));
}

unsafe extern "C" fn source(screen: *mut c_void, message: *const BatonSourceMessage) {
    // SAFETY: `screen` is a pointer that `open` gave.
    let (cell, fake) = unsafe { screen_of(screen) };
    if message.is_null() {
        fake.complain("source: a null message");
        return;
    }
    // SAFETY: not null, and the Rust side gives a struct of the interface.
    let message = unsafe { *message };
    if message.size != size_of_u32::<BatonSourceMessage>() || message.reserved != 0 {
        fake.complain(format!(
            "source: size {} reserved {}",
            message.size, message.reserved
        ));
    }
    about_screen(cell, &fake, &Call::Source(message));
}

unsafe extern "C" fn state(screen: *mut c_void, out: *mut BatonDecoderState) {
    // SAFETY: `screen` is a pointer that `open` gave.
    let (cell, fake) = unsafe { screen_of(screen) };
    about_screen(cell, &fake, &Call::State);
    if out.is_null() {
        fake.complain("state: a null struct");
        return;
    }
    // SAFETY: not null, and the Rust side gives a struct of the interface.
    let size = unsafe { (&raw const (*out).size).read() };
    if size != size_of_u32::<BatonDecoderState>() {
        fake.complain(format!("state: size {size}"));
    }
    let (decode_us, thermal) = {
        let state = fake.state();
        (state.decode_us, state.thermal)
    };
    // SAFETY: not null, and the struct is the platform's to write.
    unsafe {
        out.write(BatonDecoderState {
            size,
            decode_us,
            thermal,
            reserved: 0,
        });
    }
}

unsafe extern "C" fn ended(screen: *mut c_void) {
    // SAFETY: `screen` is a pointer that `open` gave.
    let (cell, fake) = unsafe { screen_of(screen) };
    about_screen(cell, &fake, &Call::Ended);
    if cell.ended.swap(true, Ordering::SeqCst) {
        fake.complain(format!("ended twice, on screen {}", cell.number));
    }
}

unsafe extern "C" fn release(screen: *mut c_void) {
    // SAFETY: `screen` is a pointer that `open` gave.
    let (cell, fake) = unsafe { screen_of(screen) };
    about_screen(cell, &fake, &Call::Release);
    if cell.released.swap(true, Ordering::SeqCst) {
        fake.complain(format!("released twice, on screen {}", cell.number));
    }
}

unsafe extern "C" fn sound(screen: *mut c_void, sound: *const BatonSound) {
    // SAFETY: `screen` is a pointer that `open` gave.
    let (cell, fake) = unsafe { screen_of(screen) };
    if sound.is_null() {
        fake.complain("sound: a null struct");
        return;
    }
    // SAFETY: not null, and the Rust side gives a struct of the interface.
    let sound = unsafe { *sound };
    if sound.size != size_of_u32::<BatonSound>() || sound.reserved != 0 {
        fake.complain(format!(
            "sound: size {} reserved {}",
            sound.size, sound.reserved
        ));
    }
    about_screen(cell, &fake, &Call::Sound(sound));
}

unsafe extern "C" fn play(screen: *mut c_void, play: *const BatonPlay) {
    // SAFETY: `screen` is a pointer that `open` gave.
    let (cell, fake) = unsafe { screen_of(screen) };
    if play.is_null() {
        fake.complain("play: a null struct");
        return;
    }
    // SAFETY: not null, and the Rust side gives a struct of the interface.
    let play = unsafe { *play };
    if play.size != size_of_u32::<BatonPlay>() || play.flags & !PLAY_FIRST != 0 {
        fake.complain(format!("play: size {} flags {}", play.size, play.flags));
    }
    about_screen(
        cell,
        &fake,
        &Call::Play(Played {
            flags: play.flags,
            number: play.number,
            capture_time: play.capture_time,
            lost_before: play.lost_before,
            packet: bytes(&play.packet),
        }),
    );
}

unsafe extern "C" fn stopped(screen: *mut c_void, why: u32) {
    // SAFETY: `screen` is a pointer that `open` gave.
    let (cell, fake) = unsafe { screen_of(screen) };
    about_screen(cell, &fake, &Call::Stopped(why));
}

/// Checks the room for a sentence, as the header describes it.
fn check_message(fake: &Fake, what: &str, message: BatonMessage) {
    if message.size != size_of_u32::<BatonMessage>()
        || message.cap != MESSAGE_MAX
        || message.len != 0
        || message.reserved != 0
        || message.text.is_null()
    {
        fake.complain(format!("{what}: the message is not empty: {message:?}"));
    }
}

/// A sentence of a failure, written the way a platform does: as far as there is room, with a
/// length that may claim more.
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

unsafe extern "C" fn unlock_key(
    context: *mut c_void,
    source: *const BatonUnlockSource,
    key: *mut BatonUnlockKey,
    message: *mut BatonMessage,
) -> i32 {
    // SAFETY: the context is the one `vtable` gave.
    let fake = unsafe { fake_of(context) };
    if source.is_null() || key.is_null() || message.is_null() {
        fake.complain("unlock_key: a null pointer");
        return -1;
    }
    // SAFETY: not null, and the Rust side gives structs of the interface.
    let (source, room, empty) = unsafe { (*source, *key, *message) };
    check_message(fake, "unlock_key", empty);
    if source.size != size_of_u32::<BatonUnlockSource>() || source.renew > 1 {
        fake.complain(format!(
            "unlock_key: the source is not as agreed: {source:?}"
        ));
    }
    if (room.size, room.sealing, room.len, room.reserved)
        != (size_of_u32::<BatonUnlockKey>(), 0, 0, 0)
        || room.key.is_null()
    {
        fake.complain(format!("unlock_key: the room is not empty: {room:?}"));
    }
    let id = bytes(&source.id);
    if id.len() != DEVICE_ID_BYTES as usize {
        fake.complain(format!("unlock_key: an id of {} bytes", id.len()));
    }
    let name = String::from_utf8(bytes(&source.name)).unwrap_or_else(|_| {
        fake.complain("unlock_key: a name that is not UTF-8");
        String::new()
    });
    fake.heard(
        0,
        &Call::UnlockKey {
            id,
            name,
            renew: source.renew == 1,
        },
    );
    let plan = fake.state().key.clone();
    match plan {
        KeyPlan::Gives {
            sealing,
            key: given,
        } => {
            assert!(
                given.len() <= UNLOCK_KEY_MAX as usize,
                "the plan gives more than the header has room for"
            );
            // SAFETY: the room is `UNLOCK_KEY_MAX` bytes long, as the header says, and the
            // platform's to write; the struct is the platform's to write.
            unsafe {
                room.key
                    .copy_from_nonoverlapping(given.as_ptr(), given.len());
                (&raw mut (*key).sealing).write(sealing);
                (&raw mut (*key).len).write(u32::try_from(given.len()).unwrap());
            }
            OK
        }
        KeyPlan::Fails {
            code,
            text,
            claimed_len,
        } => {
            // SAFETY: `message` is not null and its struct is the platform's to write.
            unsafe { write_failure(message, &text, claimed_len) };
            code
        }
        KeyPlan::Claims { sealing, len } => {
            // SAFETY: the struct is the platform's to write.
            unsafe {
                (&raw mut (*key).sealing).write(sealing);
                (&raw mut (*key).len).write(len);
            }
            OK
        }
    }
}

unsafe extern "C" fn unlock_open(screen: *mut c_void, question: *const BatonUnlockQuestion) -> i32 {
    // SAFETY: `screen` is a pointer that `open` gave.
    let (cell, fake) = unsafe { screen_of(screen) };
    if question.is_null() {
        fake.complain("unlock_open: a null question");
        return -1;
    }
    // SAFETY: not null, and the Rust side gives a struct of the interface.
    let question = unsafe { *question };
    if question.size != size_of_u32::<BatonUnlockQuestion>()
        || question.reserved != 0
        || question.number == 0
    {
        fake.complain(format!(
            "unlock_open: the question is not as agreed: {question:?}"
        ));
    }
    let (sealed, id) = (bytes(&question.sealed), bytes(&question.id));
    if sealed.is_empty() || sealed.len() > UNLOCK_SEALED_MAX as usize {
        fake.complain(format!(
            "unlock_open: a sealed key of {} bytes",
            sealed.len()
        ));
    }
    if id.len() != DEVICE_ID_BYTES as usize {
        fake.complain(format!("unlock_open: an id of {} bytes", id.len()));
    }
    let name = String::from_utf8(bytes(&question.name)).unwrap_or_else(|_| {
        fake.complain("unlock_open: a name that is not UTF-8");
        String::new()
    });
    about_screen(
        cell,
        &fake,
        &Call::UnlockOpen {
            number: question.number,
            sealed,
            id,
            name,
        },
    );
    fake.state().open_code
}

unsafe extern "C" fn unlock_cancel(screen: *mut c_void) {
    // SAFETY: `screen` is a pointer that `open` gave.
    let (cell, fake) = unsafe { screen_of(screen) };
    about_screen(cell, &fake, &Call::UnlockCancel);
}

// The calls of the platform into the library.

/// The person's face opened the key of the box: the answer to the question with this number.
pub(crate) fn unlock_opened(
    registry: &Registry,
    link: u64,
    number: u64,
    key: &[u8],
) -> Result<(), Error> {
    let key = BatonBytes::of(key);
    // SAFETY: a `BatonBytes` with bytes that outlive the call.
    unsafe { registry.unlock_opened(link, number, &raw const key) }
}

/// The answer to a question as the struct stands, which may be wrong: `None` is a null pointer.
///
/// # Safety
///
/// The bytes that `key` describes can be read during the call if the library is to read them: the
/// library reads no more of them than the header allows.
pub(crate) unsafe fn unlock_opened_raw(
    registry: &Registry,
    link: u64,
    number: u64,
    key: Option<&BatonBytes>,
) -> Result<(), Error> {
    let key = key.map_or(std::ptr::null(), std::ptr::from_ref);
    // SAFETY: null, or a `BatonBytes` that lives through the call, with the bytes as promised.
    unsafe { registry.unlock_opened(link, number, key) }
}

/// The question with this number was not opened, for a `FACE_` reason.
pub(crate) fn unlock_declined(
    registry: &Registry,
    link: u64,
    number: u64,
    reason: u32,
) -> Result<(), Error> {
    registry.unlock_declined(link, number, reason)
}
