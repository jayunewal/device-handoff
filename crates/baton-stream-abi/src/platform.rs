// SPDX-License-Identifier: Apache-2.0

//! Every call into the platform, and the one place that reads what the platform registered.
//!
//! This is one of the two modules of the crate that may hold `unsafe` code (the other is
//! `exports`, where the platform calls Rust). Nothing outside them calls a function that the
//! platform gave, or reads memory that it owns. The rest of the crate sees safe types:
//!
//! - [`Calls`] is the vtable the platform registered. It is made only by [`Calls::read`], which is
//!   `unsafe` because it has to trust that the pointer is the platform's struct. Everything after
//!   that is the platform's promise, written in the header: its functions may be called from any
//!   thread, with the arguments the header describes, and the pointer it gave stays valid.
//! - [`ScreenHandle`] is one screen. It is made only by [`Calls::open`] from what the platform
//!   returned, it calls the platform's functions with that pointer and no other, and it calls
//!   `release` once, when it is dropped. Its methods take what a call needs as Rust types and
//!   build the structs of the interface here, so a pointer that the platform is given always
//!   points into memory that this module keeps alive for the call.
//! - [`SharerCalls`] and [`PickedHandle`] are the same two for sharing: the vtable of the sharer,
//!   and one window that the person picked, whose pointer the platform gave with its answer
//!   ([`SharerCalls::adopt`]) and which is released once, when the handle is dropped. A window
//!   that is shared with control also asks the platform what the Mac is doing around it
//!   ([`PickedHandle::surroundings`]) and has it put what the person did there
//!   ([`PickedHandle::post`]). A platform that can give its main display without a picker also
//!   has `screen` ([`SharerCalls::screen`]), whose answer is a window of the same kind.
//!
//! Every call is made inside [`within`], which marks the thread. The functions of the interface
//! that the platform calls refuse to run on a thread that is inside a call into the platform,
//! since the session's lock is held there and a call back would wait for it.

use std::{
    cell::Cell,
    ffi::c_void,
    mem::{MaybeUninit, offset_of},
    ptr::{self, NonNull},
};

use baton_proto::limits::MAX_WINDOW_EDGE;
use baton_stream::{
    gate::{Front, Geometry, Keyboard, Post, Rect, Size},
    input::{Button, PointerAction},
};

use crate::{
    Error,
    abi::{
        ABI_VERSION, BUTTON_LEFT, BUTTON_MIDDLE, BUTTON_RIGHT, BatonBytes, BatonDecoderState,
        BatonFormat, BatonFrame, BatonFront, BatonGeometry, BatonLimits, BatonMessage, BatonPicked,
        BatonPost, BatonRect, BatonScreen, BatonSharerVTable, BatonSourceMessage,
        BatonSurroundings, BatonTakes, BatonViewerVTable, BatonWindow, CancelPickFn, ControlFn,
        DecodeFn, EndedFn, FRONT_MENU, FRONT_SAME_APPLICATION, FRONTS_MAX, KEYBOARD_ELSEWHERE,
        KEYBOARD_SHARED, KEYBOARD_UNKNOWN, LimitsFn, MESSAGE_MAX, OK, OpenFn, PICKED_DISPLAY,
        POINTER_DOWN, POINTER_MOVE, POINTER_SCROLL, POINTER_UP, POST_KEY, POST_POINTER, PickFn,
        PostFn, PrepareFn, ReleaseFn, SCREEN_NEEDS_INPUT_PERMISSION, SCREEN_NEEDS_PERMISSION,
        SCREEN_PRIVATE, SCREEN_UNAVAILABLE, ScreenFn, ShowFn, SourceFn, StartFn, StateFn,
        SurroundingsFn, size_of_u32,
    },
    registry::Registry,
    sharer::Given,
};

thread_local! {
    /// How many calls into the platform this thread is inside of.
    static INSIDE: Cell<u32> = const { Cell::new(0) };
}

/// Whether this thread is inside a call into the platform.
pub(crate) fn inside_call() -> bool {
    INSIDE.with(|depth| depth.get() > 0)
}

/// Makes a call into the platform, and marks the thread as inside one while it runs.
fn within<R>(call: impl FnOnce() -> R) -> R {
    struct Leave;
    impl Drop for Leave {
        fn drop(&mut self) {
            INSIDE.with(|depth| depth.set(depth.get().saturating_sub(1)));
        }
    }
    INSIDE.with(|depth| depth.set(depth.get().saturating_add(1)));
    let _leave = Leave;
    call()
}

/// Reads a vtable that the platform registered: a copy of the struct, if it holds the first
/// `mandatory` bytes of it. What the platform's struct lacks of this version's reads as zero, so a
/// function it lacks is `None`; and so does a function that its size cuts in two.
///
/// # Errors
///
/// The pointer is null or not aligned, the version is not the one of this library, or the size is
/// smaller than `mandatory`. A larger one than this version knows is accepted: the end of it
/// belongs to a newer version.
///
/// # Safety
///
/// `table` is either null, or points to a struct that begins with a `u32` version and a `u32`
/// size, as the two vtables of the header do, and is `size` bytes long, that nothing writes to
/// during this call. All zero bytes are a valid `T`: it holds numbers, pointers and optional
/// functions, and nothing else.
unsafe fn read_table<T: Copy>(table: *const T, mandatory: usize) -> Result<T, Error> {
    if table.is_null() || !table.is_aligned() {
        return Err(Error::Argument);
    }
    let head = table.cast::<u32>();
    // SAFETY: not null and aligned (the vtables hold pointers, so they are aligned for a `u32`
    // too), and the caller promises that a struct that has these two fields first is there; they
    // are read before anything else of it.
    let (version, size) = unsafe { (head.read(), head.add(1).read()) };
    let Ok(size) = usize::try_from(size) else {
        return Err(Error::Argument);
    };
    if version != ABI_VERSION || size < mandatory {
        return Err(Error::Argument);
    }
    // A size that ends in the middle of a field is not copied to the end: half of a pointer is
    // not a pointer. No field crosses a multiple of the alignment of the struct, so a copy that
    // ends at one never cuts a field in two.
    let copied = size.min(size_of::<T>()) & !(align_of::<T>() - 1);
    let mut whole = MaybeUninit::<T>::zeroed();
    // SAFETY: the struct is at least `size` bytes long, as the caller promises, and the bytes that
    // are copied are the first of them, no more than `T` has; they land in a `T` that was zeroed
    // first. The bytes after `T` belong to a newer version and are not read.
    unsafe {
        ptr::copy_nonoverlapping(table.cast::<u8>(), whole.as_mut_ptr().cast::<u8>(), copied);
    }
    // SAFETY: the caller promises that all zero bytes are a valid `T`, and so is a zeroed `T`
    // that has had the platform's bytes written over its start.
    Ok(unsafe { whole.assume_init() })
}

/// The functions that the platform registered, and the pointer that `limits` and `open` take.
#[derive(Clone, Copy)]
pub(crate) struct Calls {
    context: *mut c_void,
    limits: LimitsFn,
    open: OpenFn,
    decode: DecodeFn,
    show: ShowFn,
    source: SourceFn,
    state: StateFn,
    ended: EndedFn,
    release: ReleaseFn,
}

// SAFETY: a `Calls` exists only for a vtable that the platform registered, and the header's
// rules for that are the platform's promise: `limits` and `open` may be called from any thread,
// at any time, also at the same time, and `context` stays valid for as long as the process runs.
// Rust never reads through `context`.
unsafe impl Send for Calls {}
// SAFETY: as above; `Calls` has no interior mutability and is only read.
unsafe impl Sync for Calls {}

impl Calls {
    /// Reads the vtable that the platform registered. Nothing is kept of `vtable` after this: the
    /// struct is copied, and the platform may free it.
    ///
    /// # Errors
    ///
    /// The pointer is null or not aligned, the version is not the one of this library, the size
    /// is smaller than the struct that this version knows (a larger one is accepted: the end of
    /// it belongs to a newer version), or a function is missing.
    ///
    /// # Safety
    ///
    /// `vtable` is either null, or points to a struct that begins with a `u32` version and a
    /// `u32` size and is `size` bytes long, that nothing writes to during this call.
    pub(crate) unsafe fn read(vtable: *const BatonViewerVTable) -> Result<Self, Error> {
        // SAFETY: the caller's promise about `vtable` is this function's own.
        let table = unsafe { read_table(vtable, size_of::<BatonViewerVTable>()) }?;
        match (
            table.limits,
            table.open,
            table.decode,
            table.show,
            table.source,
            table.state,
            table.ended,
            table.release,
        ) {
            (
                Some(limits),
                Some(open),
                Some(decode),
                Some(show),
                Some(source),
                Some(state),
                Some(ended),
                Some(release),
            ) => Ok(Self {
                context: table.context,
                limits,
                open,
                decode,
                show,
                source,
                state,
                ended,
                release,
            }),
            _ => Err(Error::Argument),
        }
    }

    /// What the device can show, as the platform says it.
    ///
    /// # Errors
    ///
    /// The call failed.
    pub(crate) fn limits(&self) -> Result<BatonLimits, Error> {
        let mut limits = BatonLimits {
            size: size_of_u32::<BatonLimits>(),
            max_width: 0,
            max_height: 0,
            max_fps: 0,
            codecs: 0,
            reserved: 0,
        };
        // SAFETY: `limits` is a struct of the interface that lives until the call returns, and
        // the function is the platform's, called as the header says.
        let code = within(|| unsafe { (self.limits)(self.context, &raw mut limits) });
        if code == OK {
            Ok(limits)
        } else {
            Err(Error::Platform)
        }
    }

    /// Makes the decoder and the screen for `window`. `link` is the number that the platform
    /// gives back to say that the decoder failed.
    ///
    /// # Errors
    ///
    /// The platform could not: it says why, if it can. A call that succeeded and gave no pointer
    /// for the screen is a failure too, since there would be nothing to call `release` on.
    pub(crate) fn open(&self, window: &Window<'_>, link: u64) -> Result<Opened, Failure> {
        let c_window = BatonWindow {
            size: size_of_u32::<BatonWindow>(),
            width: window.width,
            height: window.height,
            control: u32::from(window.control),
            title: BatonBytes::of(window.title.as_bytes()),
            app: BatonBytes::of(window.app.as_bytes()),
        };
        let mut screen = BatonScreen {
            size: size_of_u32::<BatonScreen>(),
            reserved: 0,
            context: std::ptr::null_mut(),
            id: 0,
        };
        let mut text = [0_u8; MESSAGE_MAX as usize];
        let mut message = BatonMessage {
            size: size_of_u32::<BatonMessage>(),
            cap: MESSAGE_MAX,
            len: 0,
            reserved: 0,
            text: text.as_mut_ptr(),
        };
        // SAFETY: the three structs and the buffer behind `message.text`, which is `cap` bytes
        // long, live until the call returns; the function is the platform's, called as the header
        // says. The strings of `c_window` are borrowed from `window` for the same time.
        let code = within(|| unsafe {
            (self.open)(
                self.context,
                &raw const c_window,
                link,
                &raw mut screen,
                &raw mut message,
            )
        });
        if code != OK {
            // A failed `open` leaves nothing for `release`: the platform let go of what it made.
            return Err(Failure(sentence(&text, message.len)));
        }
        match NonNull::new(screen.context) {
            Some(context) => Ok(Opened {
                screen: ScreenHandle {
                    calls: *self,
                    context,
                },
                id: screen.id,
            }),
            None => Err(Failure(None)),
        }
    }
}

/// The sentence that the platform wrote into the buffer it was given, as far as it is text.
/// Only the buffer that was handed out is read, and only as far as `MESSAGE_MAX`, whatever the
/// platform put in the struct.
fn sentence(text: &[u8; MESSAGE_MAX as usize], len: u32) -> Option<String> {
    let len = usize::try_from(len).map_or(text.len(), |len| len.min(text.len()));
    let written = text.get(..len)?;
    // A platform that claims more than it wrote leaves zeros behind its text, and one that is
    // written in C may end its text with one.
    let written = written.split(|byte| *byte == 0).next().unwrap_or(written);
    let sentence = String::from_utf8_lossy(written);
    let sentence = sentence.trim();
    (!sentence.is_empty()).then(|| sentence.to_owned())
}

/// The window that a screen is made for.
pub(crate) struct Window<'a> {
    pub(crate) title: &'a str,
    pub(crate) app: &'a str,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) control: bool,
}

/// What the platform made for a window.
pub(crate) struct Opened {
    pub(crate) screen: ScreenHandle,
    /// The number that the user interface draws the screen with.
    pub(crate) id: i64,
}

/// Why the platform made no screen: the sentence it gave, if it gave one.
#[derive(Debug)]
pub(crate) struct Failure(pub(crate) Option<String>);

/// Why the platform gave no display without a picker.
#[derive(Debug)]
pub(crate) enum ScreenFailure {
    /// The platform gave no function for it.
    Unsupported,
    /// The system has not let the program capture the screen.
    NeedsPermission,
    /// The display was asked for as private and the system has not let the program hold the
    /// keyboard and the pointer.
    NeedsInputPermission,
    /// There is no display to give now.
    Unavailable,
    /// The platform could not, or what it gave was not whole: the sentence it wrote, if it wrote
    /// one.
    Failed(Failure),
}

/// A frame to decode.
pub(crate) struct Frame<'a> {
    pub(crate) flags: u32,
    pub(crate) number: u64,
    pub(crate) depends_on: u64,
    pub(crate) capture_time: u64,
    pub(crate) chain: u32,
    pub(crate) epoch: u32,
    /// The picture in AVCC form.
    pub(crate) sample: &'a [u8],
    pub(crate) sequence_sets: &'a [&'a [u8]],
    pub(crate) picture_sets: &'a [&'a [u8]],
}

/// One screen of the platform. Dropping it releases the screen.
pub(crate) struct ScreenHandle {
    calls: Calls,
    context: NonNull<c_void>,
}

// SAFETY: the header's rule for a screen is that its calls may come from any thread as long as
// they do not overlap. Every call but `release` takes `&self` or `&mut self`, and the screen is
// held by one owner, which the session's lock serialises, so two calls never overlap; `release`
// runs when the owner is dropped, which needs the owner to be unborrowed. Rust never reads
// through `context`.
unsafe impl Send for ScreenHandle {}

impl ScreenHandle {
    /// Gives the platform a frame to decode. Whether it took it.
    pub(crate) fn decode(&mut self, frame: &Frame<'_>) -> bool {
        let sets: Vec<BatonBytes> = frame
            .sequence_sets
            .iter()
            .chain(frame.picture_sets)
            .map(|set| BatonBytes::of(set))
            .collect();
        let c_frame = BatonFrame {
            size: size_of_u32::<BatonFrame>(),
            flags: frame.flags,
            number: frame.number,
            depends_on: frame.depends_on,
            capture_time: frame.capture_time,
            chain: frame.chain,
            epoch: frame.epoch,
            sample: BatonBytes::of(frame.sample),
            sets: if sets.is_empty() {
                std::ptr::null()
            } else {
                sets.as_ptr()
            },
            set_count: sets.len(),
            sequence_sets: frame.sequence_sets.len(),
        };
        // SAFETY: `c_frame`, `sets` and the bytes they point to live until the call returns, and
        // `context` is the screen the platform gave, not yet released (that happens in `drop`).
        let code =
            within(|| unsafe { (self.calls.decode)(self.context.as_ptr(), &raw const c_frame) });
        code == OK
    }

    /// Has the platform show the picture of frame `number`.
    pub(crate) fn show(&mut self, number: u64) {
        // SAFETY: `context` is the screen the platform gave, not yet released.
        within(|| unsafe { (self.calls.show)(self.context.as_ptr(), number) });
    }

    /// Tells the platform what the source said.
    pub(crate) fn source(&mut self, message: &BatonSourceMessage) {
        // SAFETY: `message` lives until the call returns, and `context` is the screen the
        // platform gave, not yet released.
        within(|| unsafe { (self.calls.source)(self.context.as_ptr(), &raw const *message) });
    }

    /// How the decoder is doing, as the platform says it.
    pub(crate) fn state(&self) -> BatonDecoderState {
        let mut state = BatonDecoderState {
            size: size_of_u32::<BatonDecoderState>(),
            decode_us: 0,
            thermal: 0,
            reserved: 0,
        };
        // SAFETY: `state` lives until the call returns, and `context` is the screen the platform
        // gave, not yet released. The platform writes through the pointer it is given, which
        // nothing else uses meanwhile.
        within(|| unsafe { (self.calls.state)(self.context.as_ptr(), &raw mut state) });
        state
    }

    /// Tells the platform that the session is over.
    pub(crate) fn ended(&mut self) {
        // SAFETY: `context` is the screen the platform gave, not yet released.
        within(|| unsafe { (self.calls.ended)(self.context.as_ptr()) });
    }
}

impl Drop for ScreenHandle {
    fn drop(&mut self) {
        // SAFETY: `context` is the screen the platform gave. This is the only place that
        // releases it, and it runs once; nothing calls the screen after that.
        within(|| unsafe { (self.calls.release)(self.context.as_ptr()) });
    }
}

/// Makes a call that may write the sentence of a failure, and gives what the call returned and
/// the sentence it wrote, if it did. The platform is given room for the sentence, which is read
/// as far as it fits.
fn with_message_code(call: impl FnOnce(*mut BatonMessage) -> i32) -> (i32, Option<String>) {
    let mut text = [0_u8; MESSAGE_MAX as usize];
    let mut message = BatonMessage {
        size: size_of_u32::<BatonMessage>(),
        cap: MESSAGE_MAX,
        len: 0,
        reserved: 0,
        text: text.as_mut_ptr(),
    };
    let code = call(&raw mut message);
    (code, sentence(&text, message.len))
}

/// Makes a call that may write the sentence of a failure, and gives the failure if the call
/// fails.
fn with_message(call: impl FnOnce(*mut BatonMessage) -> i32) -> Result<(), Failure> {
    match with_message_code(call) {
        (OK, _) => Ok(()),
        (_, sentence) => Err(Failure(sentence)),
    }
}

/// What the viewer of the other device takes, for `prepare`.
pub(crate) struct Takes {
    pub(crate) max_width: u32,
    pub(crate) max_height: u32,
    pub(crate) max_fps: u32,
    pub(crate) codecs: u32,
}

/// The picture that `prepare` chose, as the platform wrote it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Chosen {
    pub(crate) codec: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) fps: u32,
}

/// The functions that the platform registered to share windows, and the pointer that `control`,
/// `pick`, `cancel_pick` and `screen` take.
#[derive(Clone, Copy)]
pub(crate) struct SharerCalls {
    context: *mut c_void,
    control: ControlFn,
    pick: PickFn,
    cancel_pick: CancelPickFn,
    prepare: PrepareFn,
    start: StartFn,
    ended: EndedFn,
    release: ReleaseFn,
    /// `surroundings` and `post`, which a platform that cannot post input leaves out: both or
    /// neither.
    input: Option<(SurroundingsFn, PostFn)>,
    /// `screen`, which a platform that cannot give its screen without a picker leaves out.
    screen: Option<ScreenFn>,
}

// SAFETY: a `SharerCalls` exists only for a vtable that the platform registered, and the header's
// rules for that are the platform's promise: `control`, `pick`, `cancel_pick` and `screen` may be
// called from any thread, at any time, also at the same time, and `context` stays valid for as
// long as the process runs. Rust never reads through `context`.
unsafe impl Send for SharerCalls {}
// SAFETY: as above; `SharerCalls` has no interior mutability and is only read.
unsafe impl Sync for SharerCalls {}

impl SharerCalls {
    /// Reads the vtable that the platform registered. Nothing is kept of `vtable` after this: the
    /// struct is copied, and the platform may free it.
    ///
    /// # Errors
    ///
    /// The pointer is null or not aligned, the version is not the one of this library, the size
    /// is smaller than the part of the struct that ends at `release` (a larger one is accepted:
    /// the end of it belongs to a newer version), or one of the functions up to `release` is
    /// missing. `surroundings` and `post` may be missing; a table that has only one of them has
    /// neither. `screen` may be missing too: a platform without it cannot give its screen, and a
    /// table that ends before it, as the tables of platforms older than the call do, reads as one
    /// without it.
    ///
    /// # Safety
    ///
    /// `vtable` is either null, or points to a struct that begins with a `u32` version and a
    /// `u32` size and is `size` bytes long, that nothing writes to during this call.
    pub(crate) unsafe fn read(vtable: *const BatonSharerVTable) -> Result<Self, Error> {
        // SAFETY: the caller's promise about `vtable` is this function's own.
        let table = unsafe { read_table(vtable, offset_of!(BatonSharerVTable, surroundings)) }?;
        match (
            table.control,
            table.pick,
            table.cancel_pick,
            table.prepare,
            table.start,
            table.ended,
            table.release,
        ) {
            (
                Some(control),
                Some(pick),
                Some(cancel_pick),
                Some(prepare),
                Some(start),
                Some(ended),
                Some(release),
            ) => Ok(Self {
                context: table.context,
                control,
                pick,
                cancel_pick,
                prepare,
                start,
                ended,
                release,
                input: table.surroundings.zip(table.post),
                screen: table.screen,
            }),
            _ => Err(Error::Argument),
        }
    }

    /// Whether the person can let another device control a window that they share.
    pub(crate) fn control(&self) -> bool {
        // SAFETY: the function is the platform's, called as the header says.
        within(|| unsafe { (self.control)(self.context) }) == 1
    }

    /// Whether the platform gave the functions that put input to a window.
    pub(crate) fn can_post(&self) -> bool {
        self.input.is_some()
    }

    /// Has the platform show the picker for `request`.
    ///
    /// # Errors
    ///
    /// The platform could not: it says why, if it can.
    pub(crate) fn pick(&self, request: u64) -> Result<(), Failure> {
        // SAFETY: the message and the buffer behind it live until the call returns; the function
        // is the platform's, called as the header says.
        with_message(|message| within(|| unsafe { (self.pick)(self.context, request, message) }))
    }

    /// Tells the platform that nobody waits for the answer to `request` any more.
    pub(crate) fn cancel_pick(&self, request: u64) {
        // SAFETY: the function is the platform's, called as the header says.
        within(|| unsafe { (self.cancel_pick)(self.context, request) });
    }

    /// Whether the platform gave the function that gives its screen without a picker.
    pub(crate) fn can_screen(&self) -> bool {
        self.screen.is_some()
    }

    /// Has the platform give its main display, without the person choosing it, and keep it
    /// private from the start of the session if `private`. The call may wait for the system, so
    /// it is not made on a thread that other work waits for.
    ///
    /// A display that the platform gave is the platform's until it is taken ([`adopt`]); an
    /// answer that is not whole is taken and released here, so that it is not lost.
    ///
    /// # Errors
    ///
    /// The platform gave no function for it, or could not give the display: it says why, with
    /// the number it returns, and a sentence if it can; or what it gave is not a display that
    /// can be shared.
    ///
    /// [`adopt`]: Self::adopt
    pub(crate) fn screen(&self, private: bool) -> Result<Given, ScreenFailure> {
        let Some(screen) = self.screen else {
            return Err(ScreenFailure::Unsupported);
        };
        // What the platform fills in. Its title and its application stay empty: a display given
        // without a picker is not named, and nothing here reads bytes that the platform owns.
        let mut picked = BatonPicked {
            size: size_of_u32::<BatonPicked>(),
            width: 0,
            height: 0,
            kind: 0,
            context: ptr::null_mut(),
            title: BatonBytes {
                ptr: ptr::null(),
                len: 0,
            },
            app: BatonBytes {
                ptr: ptr::null(),
                len: 0,
            },
        };
        let flags = if private { SCREEN_PRIVATE } else { 0 };
        // SAFETY: the structs and the buffer behind the message live until the call returns; the
        // function is the platform's, called as the header says.
        let (code, text) = with_message_code(|message| {
            within(|| unsafe { screen(self.context, flags, &raw mut picked, message) })
        });
        match code {
            OK => {}
            SCREEN_NEEDS_PERMISSION => return Err(ScreenFailure::NeedsPermission),
            SCREEN_NEEDS_INPUT_PERMISSION if private => {
                return Err(ScreenFailure::NeedsInputPermission);
            }
            SCREEN_UNAVAILABLE => return Err(ScreenFailure::Unavailable),
            _ => return Err(ScreenFailure::Failed(Failure(text))),
        }
        // From here on the platform has given a display, which is released once by whoever
        // ends up with it, also when it is not whole.
        let whole = picked.kind == PICKED_DISPLAY && picked.width != 0 && picked.height != 0;
        match (NonNull::new(picked.context), whole) {
            (Some(context), true) => Ok(Given {
                context,
                display: true,
                title: String::new(),
                app: String::new(),
                width: picked.width.min(MAX_WINDOW_EDGE),
                height: picked.height.min(MAX_WINDOW_EDGE),
            }),
            (Some(context), false) => {
                drop(self.adopt(context));
                Err(ScreenFailure::Failed(Failure(None)))
            }
            (None, _) => Err(ScreenFailure::Failed(Failure(None))),
        }
    }

    /// Takes the window that the platform gave with its answer to a pick. The pointer is the
    /// platform's own: Rust only hands it back, and releases it once, when the handle is dropped.
    pub(crate) fn adopt(&self, window: NonNull<c_void>) -> PickedHandle {
        PickedHandle {
            calls: *self,
            context: window,
        }
    }
}

/// One window that the person picked. Dropping it releases the window.
pub(crate) struct PickedHandle {
    calls: SharerCalls,
    context: NonNull<c_void>,
}

// SAFETY: the header's rule for a window is that its calls may come from any thread as long as
// they do not overlap. Every call but `release` takes `&mut self`, and `release` runs when the
// handle is dropped, which needs it to be unborrowed; and a handle has one owner at a time,
// which `sharer` passes it on to (the pick, the offer, the thread that serves the session), so
// two calls never overlap. Rust never reads through `context`.
unsafe impl Send for PickedHandle {}

impl PickedHandle {
    /// Has the platform choose the picture to begin with, within `takes`. `None` if no picture
    /// within it can be made.
    pub(crate) fn prepare(&mut self, takes: &Takes) -> Option<Chosen> {
        let c_takes = BatonTakes {
            size: size_of_u32::<BatonTakes>(),
            max_width: takes.max_width,
            max_height: takes.max_height,
            max_fps: takes.max_fps,
            codecs: takes.codecs,
            reserved: 0,
        };
        let mut format = BatonFormat {
            size: size_of_u32::<BatonFormat>(),
            codec: 0,
            width: 0,
            height: 0,
            fps: 0,
            reserved: 0,
        };
        // SAFETY: both structs live until the call returns, and `context` is the window the
        // platform gave, not yet released (that happens in `drop`).
        let code = within(|| unsafe {
            (self.calls.prepare)(self.context.as_ptr(), &raw const c_takes, &raw mut format)
        });
        (code == OK).then_some(Chosen {
            codec: format.codec,
            width: format.width,
            height: format.height,
            fps: format.fps,
        })
    }

    /// Has the platform start to capture and encode the window. `link` is the number that the
    /// platform gives to the functions about the session.
    ///
    /// # Errors
    ///
    /// The platform could not: it says why, if it can.
    pub(crate) fn start(&mut self, link: u64) -> Result<(), Failure> {
        // SAFETY: the message and the buffer behind it live until the call returns, and `context`
        // is the window the platform gave, not yet released.
        with_message(|message| {
            within(|| unsafe { (self.calls.start)(self.context.as_ptr(), link, message) })
        })
    }

    /// Asks the platform what the Mac is doing around the window. `None` if the platform gave no
    /// way to ask, or the call failed.
    pub(crate) fn surroundings(&mut self) -> Option<Surroundings> {
        let (call, _) = self.calls.input?;
        let mut fronts = [BatonFront::default(); FRONTS_MAX as usize];
        // The answer that is left if the platform writes nothing is the careful one.
        let mut asked = BatonSurroundings {
            size: size_of_u32::<BatonSurroundings>(),
            locked: 1,
            on_screen: 0,
            keyboard: KEYBOARD_UNKNOWN,
            window: BatonRect::default(),
            fronts: fronts.as_mut_ptr(),
            front_cap: FRONTS_MAX,
            front_count: 0,
        };
        // SAFETY: the struct and the room for the windows in front live until the call returns,
        // and `context` is the window the platform gave, not yet released.
        let code = within(|| unsafe { call(self.context.as_ptr(), &raw mut asked) });
        if code != OK {
            return None;
        }
        let count = usize::try_from(asked.front_count)
            .ok()
            .filter(|count| *count <= fronts.len())?;
        Some(Surroundings {
            locked: asked.locked != 0,
            on_screen: asked.on_screen == 1,
            keyboard: match asked.keyboard {
                KEYBOARD_SHARED => Keyboard::Shared,
                KEYBOARD_ELSEWHERE => Keyboard::Elsewhere,
                _ => Keyboard::Unknown,
            },
            window: rect_of(&asked.window),
            fronts: fronts[..count]
                .iter()
                .map(|front| Front {
                    rect: rect_of(&front.rect),
                    same_application: front.flags & FRONT_SAME_APPLICATION != 0,
                    menu: front.flags & FRONT_MENU != 0,
                })
                .collect(),
        })
    }

    /// Has the platform put one thing that the person did to the window. `false` if the platform
    /// gave no way to, or the call failed.
    pub(crate) fn post(&mut self, post: &Post) -> bool {
        let Some((_, call)) = self.calls.input else {
            return false;
        };
        let post = c_post(post);
        // SAFETY: the struct, and the text that it points to, live until the call returns, and
        // `context` is the window the platform gave, not yet released.
        within(|| unsafe { call(self.context.as_ptr(), &raw const post) }) == OK
    }

    /// Tells the platform that the session is over.
    pub(crate) fn ended(&mut self) {
        // SAFETY: `context` is the window the platform gave, not yet released.
        within(|| unsafe { (self.calls.ended)(self.context.as_ptr()) });
    }
}

/// What the Mac is doing around a window, as the platform said.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Surroundings {
    /// The Mac is locked, or its display is off.
    pub(crate) locked: bool,
    /// The window is on screen.
    pub(crate) on_screen: bool,
    /// Whether the window has the keyboard.
    pub(crate) keyboard: Keyboard,
    /// Where the window is now.
    pub(crate) window: Rect,
    /// The windows in front of it that draw something.
    pub(crate) fronts: Vec<Front>,
}

impl Surroundings {
    /// What is assumed of a window that the platform could not say anything about: it is not
    /// known to be on the screen, so nothing is put to it.
    pub(crate) fn unknown() -> Self {
        Self {
            locked: false,
            on_screen: false,
            keyboard: Keyboard::Unknown,
            window: Rect::default(),
            fronts: Vec::new(),
        }
    }

    /// How the rules see a whole display. Everything on a display is part of what is shared, so
    /// no window lies in front of it and the keyboard is the shared one, whatever the platform
    /// said of either: a pointer reaches whatever the person at the Mac would reach there, and a
    /// key goes to whatever has the keyboard. What the platform says of the Mac being locked, and
    /// of the display being there, is believed as it is.
    pub(crate) fn of_display(mut self) -> Self {
        self.fronts.clear();
        self.keyboard = Keyboard::Shared;
        self
    }
}

pub(crate) fn rect_of(rect: &BatonRect) -> Rect {
    Rect::new(rect.x, rect.y, rect.width, rect.height)
}

/// How a picture laid out its window, as the platform wrote it. A geometry that is all zero, as
/// one that a platform without input leaves, is one that the gate does not remember.
pub(crate) fn geometry_of(geometry: &BatonGeometry) -> Geometry {
    Geometry {
        window: Size {
            width: geometry.window_width,
            height: geometry.window_height,
        },
        picture: Size {
            width: f64::from(geometry.picture_width),
            height: f64::from(geometry.picture_height),
        },
        content: rect_of(&geometry.content),
    }
}

fn button_code(button: Button) -> u32 {
    match button {
        Button::Left => BUTTON_LEFT,
        Button::Right => BUTTON_RIGHT,
        Button::Middle => BUTTON_MIDDLE,
    }
}

/// What the platform is given for `post`. The text of a key is borrowed: the struct is good for
/// as long as `post` lives.
fn c_post(post: &Post) -> BatonPost {
    let empty = BatonPost {
        size: size_of_u32::<BatonPost>(),
        kind: 0,
        x: 0.0,
        y: 0.0,
        action: 0,
        button: 0,
        dx: 0.0,
        dy: 0.0,
        modifiers: 0,
        code: 0,
        down: 0,
        repeat: 0,
        text: BatonBytes::of(&[]),
    };
    match post {
        Post::Pointer {
            at,
            modifiers,
            action,
        } => {
            let (action, button, dx, dy) = match *action {
                PointerAction::Move => (POINTER_MOVE, 0, 0.0, 0.0),
                PointerAction::Down(button) => (POINTER_DOWN, button_code(button), 0.0, 0.0),
                PointerAction::Up(button) => (POINTER_UP, button_code(button), 0.0, 0.0),
                PointerAction::Scroll { dx, dy } => (POINTER_SCROLL, 0, dx, dy),
            };
            BatonPost {
                kind: POST_POINTER,
                x: at.x,
                y: at.y,
                action,
                button,
                dx,
                dy,
                modifiers: u32::from(modifiers.bits()),
                ..empty
            }
        }
        Post::Key {
            code,
            down,
            modifiers,
            repeat,
            text,
        } => BatonPost {
            kind: POST_KEY,
            modifiers: u32::from(modifiers.bits()),
            code: u32::from(*code),
            down: u32::from(*down),
            repeat: u32::from(*repeat),
            text: text.as_deref().map_or_else(
                || BatonBytes::of(&[]),
                |text| BatonBytes::of(text.as_bytes()),
            ),
            ..empty
        },
    }
}

impl Drop for PickedHandle {
    fn drop(&mut self) {
        // SAFETY: `context` is the window the platform gave. This is the only place that releases
        // it, and it runs once; nothing calls the window after that.
        within(|| unsafe { (self.calls.release)(self.context.as_ptr()) });
    }
}

impl Registry {
    /// Registers the viewer that the platform describes in `vtable`, once. The vtable is copied,
    /// and `limits` is called before this returns.
    ///
    /// # Errors
    ///
    /// [`Error::Reentrant`] if this thread is inside a call into the platform, [`Error::Registered`]
    /// if a viewer was registered before, [`Error::Argument`] if the vtable or the limits are not
    /// valid, and [`Error::Platform`] if `limits` failed. Nothing is registered then.
    ///
    /// # Safety
    ///
    /// `vtable` is either null, or points to a struct that begins with a `u32` version and a
    /// `u32` size and is `size` bytes long, that nothing writes to during this call. Whatever
    /// the vtable's `context` points to, and its functions, are the platform's: they keep working
    /// for as long as the process runs, as the header requires.
    pub unsafe fn register_viewer(&self, vtable: *const BatonViewerVTable) -> Result<(), Error> {
        if inside_call() {
            return Err(Error::Reentrant);
        }
        // SAFETY: the caller's promise about `vtable` is this function's own.
        let calls = unsafe { Calls::read(vtable) }?;
        self.register(calls)
    }

    /// Registers the sharer that the platform describes in `vtable`, once. The vtable is copied.
    /// A platform registers a viewer, a sharer or both, in any order.
    ///
    /// # Errors
    ///
    /// [`Error::Reentrant`] if this thread is inside a call into the platform,
    /// [`Error::Registered`] if a sharer was registered before, and [`Error::Argument`] if the
    /// vtable is not valid. Nothing is registered then.
    ///
    /// # Safety
    ///
    /// `vtable` is either null, or points to a struct that begins with a `u32` version and a
    /// `u32` size and is `size` bytes long, that nothing writes to during this call. Whatever
    /// the vtable's `context` points to, and its functions, are the platform's: they keep working
    /// for as long as the process runs, as the header requires.
    pub unsafe fn register_sharer(&self, vtable: *const BatonSharerVTable) -> Result<(), Error> {
        if inside_call() {
            return Err(Error::Reentrant);
        }
        // SAFETY: the caller's promise about `vtable` is this function's own.
        let calls = unsafe { SharerCalls::read(vtable) }?;
        self.register_sharing(calls)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::{Arc, Mutex};

    use baton_stream::{gate::Point, input::Modifiers};

    use super::*;
    use crate::{
        abi::{
            CODEC_H264, FRAME_KEY, FRAME_REFERENCED, MOD_CAPS_LOCK, MOD_COMMAND, MOD_CONTROL,
            MOD_OPTION, MOD_SHIFT, SOURCE_RESUME, THERMAL_SERIOUS,
        },
        fake::{Call, Fake, OpenPlan, Watchdog},
        fake_sharer::{
            Around, AroundPlan, FakeSharer, SharerCall, key_post, left_down, pointer_post,
            read_table,
        },
    };

    /// Takes one function out of a vtable.
    type Remove = fn(&mut BatonViewerVTable);

    /// The calls of a platform that registered its vtable, which is dropped at once: only the
    /// copy is kept.
    fn registered(fake: &Arc<Fake>) -> Calls {
        let vtable = fake.vtable();
        // SAFETY: a vtable that is whole, as the fake makes it, and nothing writes to it.
        unsafe { Calls::read(&raw const vtable) }.unwrap()
    }

    fn read(table: &BatonViewerVTable) -> Option<Error> {
        // SAFETY: `table` is a struct that begins with its version and its size, and is at least
        // as long as it says in the cases that rely on that.
        unsafe { Calls::read(table) }.err()
    }

    fn window() -> Window<'static> {
        Window {
            title: "Report.pdf",
            app: "Preview",
            width: 1440,
            height: 900,
            control: true,
        }
    }

    fn open(calls: &Calls) -> Opened {
        match calls.open(&window(), 7) {
            Ok(opened) => opened,
            Err(failure) => panic!("the platform did not make a screen: {failure:?}"),
        }
    }

    fn frame<'a>(sample: &'a [u8], sets: &'a [&'a [u8]], sequence_sets: usize) -> Frame<'a> {
        let (sequence, picture) = sets.split_at(sequence_sets);
        Frame {
            flags: FRAME_KEY | FRAME_REFERENCED,
            number: 5,
            depends_on: 5,
            capture_time: 123_456,
            chain: 2,
            epoch: 1,
            sample,
            sequence_sets: sequence,
            picture_sets: picture,
        }
    }

    #[test]
    fn a_vtable_is_copied_and_its_functions_are_called() {
        let fake = Fake::new();
        let calls = registered(&fake);
        let limits = calls.limits().unwrap();
        assert_eq!(
            (
                limits.max_width,
                limits.max_height,
                limits.max_fps,
                limits.codecs
            ),
            (1920, 1200, 60, crate::abi::CODEC_H264)
        );
        assert_eq!(fake.calls(0), [Call::Limits]);
        assert!(fake.complaints().is_empty(), "{:?}", fake.complaints());
    }

    #[test]
    fn a_vtable_that_is_not_valid_is_refused() {
        let fake = Fake::new();
        let good = fake.vtable();
        assert_eq!(read(&good), None);
        // SAFETY: a null pointer is one of the cases that `read` is given.
        assert_eq!(
            unsafe { Calls::read(std::ptr::null()) }.err(),
            Some(Error::Argument)
        );
        let known = size_of_u32::<BatonViewerVTable>();
        for (what, table) in [
            ("version 0", BatonViewerVTable { version: 0, ..good }),
            ("version 1", BatonViewerVTable { version: 1, ..good }),
            ("version 3", BatonViewerVTable { version: 3, ..good }),
            ("size 0", BatonViewerVTable { size: 0, ..good }),
            ("size 8", BatonViewerVTable { size: 8, ..good }),
            (
                "one byte short",
                BatonViewerVTable {
                    size: known - 1,
                    ..good
                },
            ),
        ] {
            assert_eq!(read(&table), Some(Error::Argument), "{what}");
        }
        let missing: [(&str, Remove); 8] = [
            ("limits", |table| table.limits = None),
            ("open", |table| table.open = None),
            ("decode", |table| table.decode = None),
            ("show", |table| table.show = None),
            ("source", |table| table.source = None),
            ("state", |table| table.state = None),
            ("ended", |table| table.ended = None),
            ("release", |table| table.release = None),
        ];
        for (what, remove) in missing {
            let mut table = good;
            remove(&mut table);
            assert_eq!(read(&table), Some(Error::Argument), "no {what}");
        }
        assert!(fake.log().is_empty(), "a refused vtable was called");
    }

    #[test]
    #[allow(clippy::cast_ptr_alignment)] // the pointer is not aligned, which is the test
    fn a_vtable_that_is_not_aligned_is_refused_before_anything_is_read() {
        // A whole vtable, so that nothing is wrong with it but the address it is at.
        let fake = Fake::new();
        let good = fake.vtable();
        let mut buffer = [0_u64; 16];
        let start = buffer.as_mut_ptr().cast::<u8>();
        // SAFETY: the vtable is 80 bytes, and a buffer of 128 has room for it from the start and
        // from one byte in; the bytes are copied, so a whole vtable is where they land.
        let odd = unsafe {
            start.add(1).copy_from_nonoverlapping(
                (&raw const good).cast::<u8>(),
                size_of::<BatonViewerVTable>(),
            );
            start.add(1)
        }
        .cast::<BatonViewerVTable>();
        assert!(!odd.is_aligned());
        // SAFETY: the pointer is not aligned, which `read` checks before it reads.
        assert_eq!(unsafe { Calls::read(odd) }.err(), Some(Error::Argument));

        // The same bytes where they are aligned are taken.
        // SAFETY: as above, and now the vtable is whole at an aligned address.
        let aligned = unsafe {
            start.copy_from_nonoverlapping(
                (&raw const good).cast::<u8>(),
                size_of::<BatonViewerVTable>(),
            );
            start
        }
        .cast::<BatonViewerVTable>();
        assert!(aligned.is_aligned());
        // SAFETY: a whole vtable that nothing writes to.
        assert!(unsafe { Calls::read(aligned) }.is_ok());
    }

    #[test]
    fn a_vtable_from_a_newer_version_is_read_as_far_as_this_version_knows_it() {
        #[repr(C)]
        struct Longer {
            table: BatonViewerVTable,
            newer: [u64; 2],
        }
        let fake = Fake::new();
        let longer = Longer {
            table: BatonViewerVTable {
                size: size_of_u32::<Longer>(),
                ..fake.vtable()
            },
            newer: [u64::MAX; 2],
        };
        // SAFETY: `longer` starts with a vtable and is as long as its size says.
        let calls = unsafe { Calls::read((&raw const longer).cast()) }.unwrap();
        assert!(calls.limits().is_ok());
        assert_eq!(longer.newer, [u64::MAX; 2]);
    }

    #[test]
    fn a_limits_call_that_fails_is_a_platform_failure() {
        let fake = Fake::new();
        let calls = registered(&fake);
        fake.fail_limits(-9);
        assert_eq!(calls.limits().err(), Some(Error::Platform));
    }

    #[test]
    fn a_window_goes_to_the_platform_and_a_screen_comes_back() {
        let fake = Fake::new();
        let calls = registered(&fake);
        let opened = open(&calls);
        assert_eq!(opened.id, 100);
        assert_eq!(
            fake.calls(0),
            [Call::Open {
                title: "Report.pdf".to_owned(),
                app: "Preview".to_owned(),
                width: 1440,
                height: 900,
                control: 1,
                link: 7,
            }]
        );
        assert_eq!(fake.live_screens(), 1);
        drop(opened);
        assert_eq!(fake.calls(1), [Call::Release]);
        assert_eq!(fake.live_screens(), 0);
        assert!(fake.complaints().is_empty(), "{:?}", fake.complaints());
    }

    #[test]
    fn a_window_without_a_title_or_an_application_is_empty_text() {
        let fake = Fake::new();
        let calls = registered(&fake);
        let opened = calls
            .open(
                &Window {
                    title: "",
                    app: "",
                    width: 1,
                    height: 1,
                    control: false,
                },
                1,
            )
            .ok()
            .unwrap();
        assert_eq!(
            fake.calls(0),
            [Call::Open {
                title: String::new(),
                app: String::new(),
                width: 1,
                height: 1,
                control: 0,
                link: 1,
            }]
        );
        drop(opened);
        assert!(fake.complaints().is_empty(), "{:?}", fake.complaints());
    }

    #[test]
    fn the_sentence_of_a_failed_open_is_read_as_far_as_the_room_it_was_given() {
        let long = vec![b'x'; 1000];
        let cases: [(&str, &[u8], u32, Option<&str>); 9] = [
            (
                "plain",
                b"No room for the picture.",
                25,
                Some("No room for the picture."),
            ),
            ("spaces around it", b"  Not now.  ", 12, Some("Not now.")),
            ("nothing", b"", 0, None),
            ("only spaces", b"   ", 3, None),
            ("not UTF-8", &[0xff, b'a'], 2, Some("\u{fffd}a")),
            ("it ends in a zero", b"Stopped.\0", 9, Some("Stopped.")),
            ("a zero in the middle", b"ab\0cd", 5, Some("ab")),
            ("it claims more than it wrote", b"abc", 1000, Some("abc")),
            ("it claims more than there is room", &long, 100_000, None),
        ];
        for (what, text, claimed_len, expected) in cases {
            let fake = Fake::new();
            let calls = registered(&fake);
            fake.plan_open(OpenPlan::Fail {
                text: text.to_vec(),
                claimed_len,
            });
            let failure = calls.open(&window(), 1).err().expect(what);
            if what == "it claims more than there is room" {
                // Only the room that was handed out is read: 256 bytes, and no more.
                assert_eq!(failure.0.as_deref().map(str::len), Some(256), "{what}");
            } else {
                assert_eq!(failure.0.as_deref(), expected, "{what}");
            }
            assert_eq!(fake.screens_made(), 0, "{what}");
            assert!(
                fake.complaints().is_empty(),
                "{what}: {:?}",
                fake.complaints()
            );
        }
    }

    #[test]
    fn an_open_that_worked_and_gave_no_pointer_is_a_failure() {
        let fake = Fake::new();
        let calls = registered(&fake);
        fake.plan_open(OpenPlan::NoPointer);
        let failure = calls.open(&window(), 1).err().unwrap();
        assert_eq!(failure.0, None);
    }

    #[test]
    fn a_screen_is_given_what_the_session_gives_it_and_released_once() {
        let fake = Fake::new();
        let calls = registered(&fake);
        let mut opened = open(&calls);
        let sample = [0, 0, 0, 3, 0x65, 1, 2];
        let sps: &[u8] = &[0x67, 1];
        let pps: &[u8] = &[0x68, 2];
        let sets = [sps, pps];
        assert!(opened.screen.decode(&frame(&sample, &sets, 1)));
        opened.screen.show(5);
        let mut message = BatonSourceMessage {
            size: size_of_u32::<BatonSourceMessage>(),
            kind: SOURCE_RESUME,
            epoch: 0,
            codec: 0,
            width: 0,
            height: 0,
            fps: 0,
            reason: 0,
            count: 0,
            reserved: 0,
        };
        opened.screen.source(&message);
        fake.set_state(2500, THERMAL_SERIOUS);
        let state = opened.screen.state();
        assert_eq!((state.decode_us, state.thermal), (2500, THERMAL_SERIOUS));
        opened.screen.ended();
        drop(opened);

        message.size = size_of_u32::<BatonSourceMessage>();
        assert_eq!(
            fake.calls(1),
            [
                Call::Decode(crate::fake::Decoded {
                    flags: FRAME_KEY | FRAME_REFERENCED,
                    number: 5,
                    depends_on: 5,
                    capture_time: 123_456,
                    chain: 2,
                    epoch: 1,
                    sample: sample.to_vec(),
                    sets: vec![sps.to_vec(), pps.to_vec()],
                    sequence_sets: 1,
                }),
                Call::Show(5),
                Call::Source(message),
                Call::State,
                Call::Ended,
                Call::Release,
            ]
        );
        assert!(fake.complaints().is_empty(), "{:?}", fake.complaints());
    }

    #[test]
    fn a_frame_without_parameter_sets_is_given_with_no_pointer_for_them() {
        let fake = Fake::new();
        let calls = registered(&fake);
        let mut opened = open(&calls);
        let sample = [0, 0, 0, 2, 0x41, 9];
        let mut delta = frame(&sample, &[], 0);
        delta.flags = 0;
        assert!(opened.screen.decode(&delta));
        let Some(Call::Decode(decoded)) = fake.calls(1).into_iter().next() else {
            panic!("the platform was not asked to decode");
        };
        assert_eq!((decoded.sets.len(), decoded.sequence_sets), (0, 0));
        assert_eq!(decoded.flags, 0);
    }

    #[test]
    fn a_frame_that_the_platform_refuses_is_refused() {
        let fake = Fake::new();
        let calls = registered(&fake);
        let mut opened = open(&calls);
        fake.fail_decode(-4);
        let sample = [0, 0, 0, 2, 0x41, 9];
        assert!(!opened.screen.decode(&frame(&sample, &[], 0)));
        fake.fail_decode(0);
        assert!(opened.screen.decode(&frame(&sample, &[], 0)));
    }

    #[test]
    fn a_thread_is_inside_a_call_for_as_long_as_the_call_lasts() {
        let fake = Fake::new();
        let inside = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&inside);
        fake.on_call(move |call| seen.lock().unwrap().push((call.clone(), inside_call())));
        assert!(!inside_call());
        let calls = registered(&fake);
        calls.limits().unwrap();
        let mut opened = open(&calls);
        let sample = [0, 0, 0, 2, 0x41, 9];
        opened.screen.decode(&frame(&sample, &[], 0));
        opened.screen.show(0);
        opened.screen.state();
        opened.screen.ended();
        drop(opened);
        assert!(!inside_call());
        let seen = inside.lock().unwrap();
        assert_eq!(seen.len(), 7);
        for (call, inside) in seen.iter() {
            assert!(inside, "{call:?} was made outside of the marking");
        }
    }

    #[test]
    fn calls_nest_and_the_mark_is_let_go_of_when_the_outer_call_ends() {
        let nested = within(|| {
            let outer = inside_call();
            let inner = within(inside_call);
            // Still inside after the inner call returned.
            (outer, inner, inside_call())
        });
        assert_eq!(nested, (true, true, true));
        assert!(!inside_call());
    }

    #[test]
    fn a_registration_from_inside_a_call_is_refused_and_nothing_waits() {
        let fake = Fake::new();
        let registry = Arc::new(Registry::new());
        let seen = Arc::new(Mutex::new(None));
        let (inner, outcome) = (Arc::clone(&registry), Arc::clone(&seen));
        let other = Fake::new();
        fake.on_call(move |call| {
            if *call == Call::Limits {
                let table = other.vtable();
                // A registration that was let in would wait for the one that is being made.
                let _watch = Watchdog::start("a registration from inside a call");
                // SAFETY: a whole vtable that nothing writes to.
                *outcome.lock().unwrap() = Some(unsafe { inner.register_viewer(&raw const table) });
            }
        });
        let table = fake.vtable();
        // SAFETY: a whole vtable that nothing writes to.
        unsafe { registry.register_viewer(&raw const table) }.unwrap();
        assert_eq!(*seen.lock().unwrap(), Some(Err(Error::Reentrant)));
        assert!(registry.viewer().is_some());
    }

    /// A window of the sharing platform of the tests that was prepared and started, which is
    /// when the platform is asked about the Mac.
    fn started(fake: &Arc<FakeSharer>, calls: &SharerCalls) -> PickedHandle {
        let mut window = calls.adopt(NonNull::new(fake.new_window()).unwrap());
        let takes = Takes {
            max_width: 1920,
            max_height: 1080,
            max_fps: 60,
            codecs: CODEC_H264,
        };
        window.prepare(&takes).unwrap();
        window.start(1).unwrap();
        window
    }

    #[test]
    fn the_numbers_of_the_header_are_the_bits_and_the_buttons_that_the_rules_use() {
        // The modifiers go across as the byte that the rules hold them in.
        assert_eq!(u32::from(Modifiers::SHIFT.bits()), MOD_SHIFT);
        assert_eq!(u32::from(Modifiers::CONTROL.bits()), MOD_CONTROL);
        assert_eq!(u32::from(Modifiers::OPTION.bits()), MOD_OPTION);
        assert_eq!(u32::from(Modifiers::COMMAND.bits()), MOD_COMMAND);
        assert_eq!(u32::from(Modifiers::CAPS_LOCK.bits()), MOD_CAPS_LOCK);
        assert_eq!(button_code(Button::Left), BUTTON_LEFT);
        assert_eq!(button_code(Button::Right), BUTTON_RIGHT);
        assert_eq!(button_code(Button::Middle), BUTTON_MIDDLE);
    }

    #[test]
    fn every_kind_of_post_reaches_the_platform_as_the_header_describes_it() {
        let fake = FakeSharer::new();
        let mut window = started(&fake, &fake.platform());
        let every_key = Modifiers::SHIFT
            | Modifiers::CONTROL
            | Modifiers::OPTION
            | Modifiers::COMMAND
            | Modifiers::CAPS_LOCK;
        let mut posts = Vec::new();
        for modifiers in [Modifiers::NONE, Modifiers::OPTION, every_key] {
            // A window on a display to the left of the main one is at negative places.
            let at = Point { x: -123.5, y: 4.25 };
            for action in [
                PointerAction::Move,
                PointerAction::Down(Button::Left),
                PointerAction::Down(Button::Right),
                PointerAction::Down(Button::Middle),
                PointerAction::Up(Button::Left),
                PointerAction::Up(Button::Right),
                PointerAction::Up(Button::Middle),
                PointerAction::Scroll { dx: 12.5, dy: -3.0 },
                PointerAction::Scroll { dx: 0.0, dy: 7.0 },
                PointerAction::Scroll { dx: -1.5, dy: 0.0 },
            ] {
                posts.push(Post::Pointer {
                    at,
                    modifiers,
                    action,
                });
            }
            for (code, down, repeat, text) in [
                (0x04, true, false, Some("a")),
                (0x04, true, true, Some("a")),
                (0x04, false, false, None),
                (0xE7, true, false, None),
                (0, true, false, Some("h\u{e9}llo \u{1f600} \u{4e16}")),
            ] {
                posts.push(Post::Key {
                    code,
                    down,
                    modifiers,
                    repeat,
                    text: text.map(str::to_owned),
                });
            }
        }
        for post in &posts {
            assert!(window.post(post), "{post:?}");
        }
        assert_eq!(fake.posts(1), posts);
        assert_eq!(fake.complaints(), Vec::<String>::new());
    }

    #[test]
    fn the_surroundings_of_a_window_are_read_as_the_header_says() {
        let fake = FakeSharer::new();
        let mut window = started(&fake, &fake.platform());
        let in_front = |x: f64, flags: u32| BatonFront {
            rect: BatonRect {
                x,
                y: -2.0,
                width: 10.0,
                height: 20.0,
            },
            flags,
            reserved: 0,
        };
        let front = |x: f64, same_application, menu| Front {
            rect: Rect::new(x, -2.0, 10.0, 20.0),
            same_application,
            menu,
        };

        fake.set_around(Around {
            locked: false,
            on_screen: true,
            keyboard: KEYBOARD_ELSEWHERE,
            window: BatonRect {
                x: 100.0,
                y: 50.0,
                width: 400.0,
                height: 300.0,
            },
            fronts: vec![
                in_front(1.0, 0),
                in_front(2.0, FRONT_SAME_APPLICATION),
                in_front(3.0, FRONT_MENU),
                in_front(4.0, FRONT_SAME_APPLICATION | FRONT_MENU),
                // A bit that this version does not know is nothing to it.
                in_front(5.0, !(FRONT_SAME_APPLICATION | FRONT_MENU)),
            ],
        });
        assert_eq!(
            window.surroundings(),
            Some(Surroundings {
                locked: false,
                on_screen: true,
                keyboard: Keyboard::Elsewhere,
                window: Rect::new(100.0, 50.0, 400.0, 300.0),
                fronts: vec![
                    front(1.0, false, false),
                    front(2.0, true, false),
                    front(3.0, false, true),
                    front(4.0, true, true),
                    front(5.0, false, false),
                ],
            })
        );

        assert_eq!(fake.complaints(), Vec::<String>::new());
    }

    #[test]
    fn the_numbers_that_the_platform_writes_for_the_mac_are_read_with_care() {
        let fake = FakeSharer::new();
        let mut window = started(&fake, &fake.platform());
        let in_front = |x: f64| BatonFront {
            rect: BatonRect {
                x,
                y: -2.0,
                width: 10.0,
                height: 20.0,
            },
            flags: 0,
            reserved: 0,
        };

        // Who has the keyboard: a number that this version does not know is not "the window".
        for (keyboard, expected) in [
            (KEYBOARD_SHARED, Keyboard::Shared),
            (KEYBOARD_ELSEWHERE, Keyboard::Elsewhere),
            (KEYBOARD_UNKNOWN, Keyboard::Unknown),
            (3, Keyboard::Unknown),
            (u32::MAX, Keyboard::Unknown),
        ] {
            fake.plan_around(AroundPlan::Numbers {
                locked: 0,
                on_screen: 1,
                keyboard,
            });
            let around = window.surroundings().unwrap();
            assert_eq!(around.keyboard, expected, "keyboard {keyboard}");
        }

        // The Mac is locked unless the platform says that it is not, and the window is on the
        // screen only if the platform says so with the number that means it.
        for (locked, on_screen, expected) in [
            (0, 1, (false, true)),
            (1, 1, (true, true)),
            (2, 1, (true, true)),
            (u32::MAX, 1, (true, true)),
            (0, 0, (false, false)),
            (0, 2, (false, false)),
            (0, u32::MAX, (false, false)),
        ] {
            fake.plan_around(AroundPlan::Numbers {
                locked,
                on_screen,
                keyboard: KEYBOARD_SHARED,
            });
            let around = window.surroundings().unwrap();
            assert_eq!(
                (around.locked, around.on_screen),
                expected,
                "locked {locked}, on screen {on_screen}"
            );
        }

        // A platform that writes nothing leaves the careful answer.
        fake.plan_around(AroundPlan::WritesNothing);
        assert_eq!(
            window.surroundings(),
            Some(Surroundings {
                locked: true,
                on_screen: false,
                keyboard: Keyboard::Unknown,
                window: Rect::default(),
                fronts: Vec::new(),
            })
        );

        // As many windows in front as there is room for are read, and more than that are not
        // believed.
        let crowd = Around {
            fronts: (0..FRONTS_MAX).map(|at| in_front(f64::from(at))).collect(),
            ..Around::default()
        };
        fake.plan_around(AroundPlan::Claims(crowd.clone(), FRONTS_MAX));
        let around = window.surroundings().unwrap();
        assert_eq!(around.fronts.len(), FRONTS_MAX as usize);
        fake.plan_around(AroundPlan::Claims(crowd.clone(), FRONTS_MAX + 1));
        assert_eq!(window.surroundings(), None);
        fake.plan_around(AroundPlan::Claims(crowd, u32::MAX));
        assert_eq!(window.surroundings(), None);

        // A call that fails says nothing.
        fake.plan_around(AroundPlan::Fails(-1));
        assert_eq!(window.surroundings(), None);
        assert_eq!(fake.complaints(), Vec::<String>::new());
    }

    #[test]
    fn a_platform_that_posts_nothing_is_not_asked_and_cannot_be_told() {
        let fake = FakeSharer::new();
        // Neither function: a table that ends before them, or one that has none of them.
        let ends_early = fake.vtable_without_input();
        let none = BatonSharerVTable {
            surroundings: None,
            post: None,
            ..fake.vtable()
        };
        // Only one of them is as good as none.
        let only_asks = BatonSharerVTable {
            post: None,
            ..fake.vtable()
        };
        let only_posts = BatonSharerVTable {
            surroundings: None,
            ..fake.vtable()
        };
        for (what, table) in [
            ("a table that ends early", ends_early),
            ("a table with neither", none),
            ("a table that can only ask", only_asks),
            ("a table that can only post", only_posts),
        ] {
            let calls = read_table(&table).unwrap();
            assert!(!calls.can_post(), "{what}");
            let mut window = started(&fake, &calls);
            assert_eq!(window.surroundings(), None, "{what}");
            assert!(!window.post(&left_down(1.0, 2.0)), "{what}");
        }
        let asked =
            |call: &SharerCall| matches!(call, SharerCall::Surroundings | SharerCall::Post(_));
        assert_eq!(fake.count(asked), 0);

        // The whole table can.
        let calls = fake.platform();
        assert!(calls.can_post());
        let mut window = started(&fake, &calls);
        assert!(window.surroundings().is_some());
        assert!(window.post(&pointer_post(1.0, 2.0, PointerAction::Move)));
        assert!(window.post(&key_post(4, true, None)));
        assert_eq!(fake.count(asked), 3);
    }
}
