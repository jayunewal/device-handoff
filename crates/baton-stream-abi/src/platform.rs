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
//!   ([`SharerCalls::adopt`]) and which is released once, when the handle is dropped.
//!
//! Every call is made inside [`within`], which marks the thread. The functions of the interface
//! that the platform calls refuse to run on a thread that is inside a call into the platform,
//! since the session's lock is held there and a call back would wait for it.

use std::{cell::Cell, ffi::c_void, ptr::NonNull};

use crate::{
    Error,
    abi::{
        ABI_VERSION, BatonBytes, BatonDecoderState, BatonFormat, BatonFrame, BatonLimits,
        BatonMessage, BatonScreen, BatonSharerVTable, BatonSourceMessage, BatonTakes,
        BatonViewerVTable, BatonWindow, CancelPickFn, ControlFn, DecodeFn, EndedFn, LimitsFn,
        MESSAGE_MAX, OK, OpenFn, PickFn, PrepareFn, ReleaseFn, ShowFn, SourceFn, StartFn, StateFn,
        size_of_u32,
    },
    registry::Registry,
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

/// Reads a vtable that the platform registered: a copy of the struct, if it is whole.
///
/// # Errors
///
/// The pointer is null or not aligned, the version is not the one of this library, or the size is
/// smaller than the struct that this version knows. A larger one is accepted: the end of it
/// belongs to a newer version.
///
/// # Safety
///
/// `table` is either null, or points to a struct that begins with a `u32` version and a `u32`
/// size, as the two vtables of the header do, and is `size` bytes long, that nothing writes to
/// during this call.
unsafe fn read_table<T: Copy>(table: *const T) -> Result<T, Error> {
    if table.is_null() || !table.is_aligned() {
        return Err(Error::Argument);
    }
    let head = table.cast::<u32>();
    // SAFETY: not null and aligned (the vtables hold pointers, so they are aligned for a `u32`
    // too), and the caller promises that a struct that has these two fields first is there; they
    // are read before anything else of it.
    let (version, size) = unsafe { (head.read(), head.add(1).read()) };
    if version != ABI_VERSION || usize::try_from(size).map_or(true, |size| size < size_of::<T>()) {
        return Err(Error::Argument);
    }
    // SAFETY: the struct is at least as long as `T` (the caller promises it is `size` bytes), and
    // the pointer is aligned. The bytes after `T` belong to a newer version and are not read.
    Ok(unsafe { table.read() })
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
        let table = unsafe { read_table(vtable) }?;
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

/// Makes a call that may write the sentence of a failure, and gives the failure if the call
/// fails. The platform is given room for the sentence, which is read as far as it fits.
fn with_message(call: impl FnOnce(*mut BatonMessage) -> i32) -> Result<(), Failure> {
    let mut text = [0_u8; MESSAGE_MAX as usize];
    let mut message = BatonMessage {
        size: size_of_u32::<BatonMessage>(),
        cap: MESSAGE_MAX,
        len: 0,
        reserved: 0,
        text: text.as_mut_ptr(),
    };
    if call(&raw mut message) == OK {
        Ok(())
    } else {
        Err(Failure(sentence(&text, message.len)))
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
/// `pick` and `cancel_pick` take.
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
}

// SAFETY: a `SharerCalls` exists only for a vtable that the platform registered, and the header's
// rules for that are the platform's promise: `control`, `pick` and `cancel_pick` may be called
// from any thread, at any time, also at the same time, and `context` stays valid for as long as
// the process runs. Rust never reads through `context`.
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
    /// is smaller than the struct that this version knows (a larger one is accepted: the end of
    /// it belongs to a newer version), or a function is missing.
    ///
    /// # Safety
    ///
    /// `vtable` is either null, or points to a struct that begins with a `u32` version and a
    /// `u32` size and is `size` bytes long, that nothing writes to during this call.
    pub(crate) unsafe fn read(vtable: *const BatonSharerVTable) -> Result<Self, Error> {
        // SAFETY: the caller's promise about `vtable` is this function's own.
        let table = unsafe { read_table(vtable) }?;
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
            }),
            _ => Err(Error::Argument),
        }
    }

    /// Whether the person can let another device control a window that they share.
    pub(crate) fn control(&self) -> bool {
        // SAFETY: the function is the platform's, called as the header says.
        within(|| unsafe { (self.control)(self.context) }) == 1
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
// which `sharer` passes it on to (the pick, the offer, the task that waits for the end of the
// session), so two calls never overlap. Rust never reads through `context`.
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

    /// Tells the platform that the session is over.
    pub(crate) fn ended(&mut self) {
        // SAFETY: `context` is the window the platform gave, not yet released.
        within(|| unsafe { (self.calls.ended)(self.context.as_ptr()) });
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

    use super::*;
    use crate::{
        abi::{FRAME_KEY, FRAME_REFERENCED, SOURCE_RESUME, THERMAL_SERIOUS},
        fake::{Call, Fake, OpenPlan, Watchdog},
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
            ("version 2", BatonViewerVTable { version: 2, ..good }),
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
}
