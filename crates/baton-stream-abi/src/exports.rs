// SPDX-License-Identifier: Apache-2.0

//! The functions of the interface that the platform calls: the C symbols of
//! `include/baton_stream_abi.h`.
//!
//! This is one of the two modules of the crate that may hold `unsafe` code (the other is
//! `platform`, where Rust calls the platform). Each function checks what it is given, does its
//! work in the registry of the process, and answers with a number: nothing a platform passes can
//! make one of them do more than answer with an error. A panic is caught and answered the same
//! way, since one that unwound into the platform's frames would end the process.
//!
//! What a platform passes is read here and nowhere else: a struct is copied only if it states a
//! size that holds the part this version knows, a run of bytes only up to the most that its use
//! allows, a pointer that is written through is checked before any work is done and written only
//! when the function succeeds, and nothing that a pointer led to is kept past the call.

use std::{
    mem::{MaybeUninit, offset_of},
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::{self, NonNull},
    slice,
};

use baton_proto::limits::{
    MAX_NAME_BYTES, MAX_PICTURE_BYTES, MAX_STREAM_TITLE_BYTES, MAX_WINDOW_EDGE,
};

use crate::{
    Error,
    abi::{
        ABI_VERSION, BatonBytes, BatonFormat, BatonPicked, BatonPicture, BatonSharerVTable,
        BatonViewerVTable, FRAME_REFERENCED, MESSAGE_MAX, PICKED_DISPLAY, PICKED_WINDOW,
        SET_BYTES_MAX, SETS_MAX,
    },
    platform::{Chosen, geometry_of, inside_call},
    registry::{Registry, global},
    sharer::{Given, Made, PickAnswer, SourceLink},
};

/// Runs `call` and gives the number that the platform is told: its result, or the number of a
/// panic.
fn answer(call: impl FnOnce() -> Result<(), Error>) -> i32 {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(result) => Error::status(result),
        Err(_) => Error::Panic.code(),
    }
}

/// The version of the interface that this library was built with.
#[unsafe(no_mangle)]
pub extern "C" fn baton_stream_abi_version() -> u32 {
    ABI_VERSION
}

/// Registers the platform's viewer, once, before the app starts. See the header.
///
/// # Safety
///
/// `vtable` is either null, or points to a struct that begins with a `u32` version and a `u32`
/// size and is `size` bytes long, as the header says.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn baton_stream_register_viewer(vtable: *const BatonViewerVTable) -> i32 {
    // SAFETY: the caller's promise about `vtable` is this function's own.
    answer(|| unsafe { global().register_viewer(vtable) })
}

/// The decoder of the screen that `link` was given for failed. See the header.
#[unsafe(no_mangle)]
pub extern "C" fn baton_stream_link_decode_failed(link: u64) -> i32 {
    answer(|| global().link_decode_failed(link))
}

/// Registers the platform's sharer, once, before the app starts. See the header.
///
/// # Safety
///
/// `vtable` is either null, or points to a struct that begins with a `u32` version and a `u32`
/// size and is `size` bytes long, as the header says.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn baton_stream_register_sharer(vtable: *const BatonSharerVTable) -> i32 {
    // SAFETY: the caller's promise about `vtable` is this function's own.
    answer(|| unsafe { global().register_sharer(vtable) })
}

/// The answer to a pick. See the header.
///
/// # Safety
///
/// Each of `picked` and `failure` is either null, or points to what the header says: a
/// `BatonPicked` that begins with its size and is that long, and a `BatonBytes`, with the bytes
/// that it describes, which nothing writes to during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn baton_stream_pick_done(
    request: u64,
    picked: *const BatonPicked,
    failure: *const BatonBytes,
) -> i32 {
    // SAFETY: the caller's promise about the pointers is this function's own.
    answer(|| unsafe { global().answer_pick(request, picked, failure) })
}

/// The microseconds since the session of `link` began. See the header.
///
/// # Safety
///
/// `micros` is either null, or points to a `u64` that can be written during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn baton_stream_source_clock(link: u64, micros: *mut u64) -> i32 {
    // SAFETY: the caller's promise about `micros` is this function's own.
    answer(|| unsafe { global().source_clock(link, micros) })
}

/// Asks whether to make a picture. See the header.
///
/// # Safety
///
/// `kind` is either null, or points to a `u32` that can be written during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn baton_stream_source_begin(link: u64, kind: *mut u32) -> i32 {
    // SAFETY: the caller's promise about `kind` is this function's own.
    answer(|| unsafe { global().source_begin(link, kind) })
}

/// The picture that `begin` announced could not be made. See the header.
#[unsafe(no_mangle)]
pub extern "C" fn baton_stream_source_cancel(link: u64) -> i32 {
    answer(|| global().source_cancel(link))
}

/// Whether the next picture has to be something other than a delta. See the header.
///
/// # Safety
///
/// `pending` is either null, or points to a `u32` that can be written during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn baton_stream_source_recovery_pending(link: u64, pending: *mut u32) -> i32 {
    // SAFETY: the caller's promise about `pending` is this function's own.
    answer(|| unsafe { global().source_recovery_pending(link, pending) })
}

/// The picture that `begin` announced was made: sends it. See the header.
///
/// # Safety
///
/// `picture` is either null, or points to a `BatonPicture` that begins with its size and is that
/// long, whose `sample` and `sets` describe memory that can be read during the call and that
/// nothing writes to meanwhile; `outcome` and `number` are either null, or point to a `u32` and a
/// `u64` that can be written during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn baton_stream_source_sent(
    link: u64,
    picture: *const BatonPicture,
    outcome: *mut u32,
    number: *mut u64,
) -> i32 {
    // SAFETY: the caller's promise about the pointers is this function's own.
    answer(|| unsafe { global().source_sent(link, picture, outcome, number) })
}

/// The window changed. See the header.
#[unsafe(no_mangle)]
pub extern "C" fn baton_stream_source_changed(link: u64, capture_time: u64) -> i32 {
    answer(|| global().source_changed(link, capture_time))
}

/// The size of the picture changed. See the header.
///
/// # Safety
///
/// `format` is either null, or points to a `BatonFormat` that begins with its size and is that
/// long.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn baton_stream_source_new_epoch(
    link: u64,
    format: *const BatonFormat,
) -> i32 {
    // SAFETY: the caller's promise about `format` is this function's own.
    answer(|| unsafe { global().source_new_epoch(link, format) })
}

/// Stops the pictures for a while. See the header.
#[unsafe(no_mangle)]
pub extern "C" fn baton_stream_source_pause(link: u64, reason: u32) -> i32 {
    answer(|| global().source_pause(link, reason))
}

/// The pictures go on. See the header.
#[unsafe(no_mangle)]
pub extern "C" fn baton_stream_source_resume(link: u64) -> i32 {
    answer(|| global().source_resume(link))
}

/// Ends the session. See the header.
///
/// # Safety
///
/// `sentence` is either null, or points to a `BatonBytes` with the bytes that it describes, which
/// nothing writes to during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn baton_stream_source_end(link: u64, sentence: *const BatonBytes) -> i32 {
    // SAFETY: the caller's promise about `sentence` is this function's own.
    answer(|| unsafe { global().source_end(link, sentence) })
}

// What the functions above do, as methods of a registry, so that a test can call them on one of
// its own with the same pointers that a platform passes.
impl Registry {
    /// The answer of the platform to the pick that `request` was given for. An answer that
    /// cannot be read ends the wait all the same: the person is waiting for a window.
    ///
    /// # Safety
    ///
    /// As for [`baton_stream_pick_done`].
    pub(crate) unsafe fn answer_pick(
        &self,
        request: u64,
        picked: *const BatonPicked,
        failure: *const BatonBytes,
    ) -> Result<(), Error> {
        if inside_call() {
            return Err(Error::Reentrant);
        }
        // SAFETY: the caller's promise about the pointers is this function's own.
        let read = unsafe { read_answer(picked, failure) };
        self.picks.answer(request, read)
    }

    /// The microseconds since the session of `link` began.
    ///
    /// # Safety
    ///
    /// As for [`baton_stream_source_clock`].
    pub(crate) unsafe fn source_clock(&self, link: u64, micros: *mut u64) -> Result<(), Error> {
        let source = self.source(link)?;
        let micros = writable(micros)?;
        // SAFETY: `micros` is writable for the call, as the caller promises, and was checked.
        unsafe { micros.write(source.clock()) };
        Ok(())
    }

    /// Whether to make a picture, and which kind.
    ///
    /// # Safety
    ///
    /// As for [`baton_stream_source_begin`].
    pub(crate) unsafe fn source_begin(&self, link: u64, kind: *mut u32) -> Result<(), Error> {
        let source = self.source(link)?;
        let kind = writable(kind)?;
        // SAFETY: `kind` is writable for the call, as the caller promises, and was checked.
        unsafe { kind.write(source.begin()) };
        Ok(())
    }

    /// The picture that `begin` announced could not be made.
    pub(crate) fn source_cancel(&self, link: u64) -> Result<(), Error> {
        self.source(link)?.cancel();
        Ok(())
    }

    /// Whether the next picture has to be something other than a delta.
    ///
    /// # Safety
    ///
    /// As for [`baton_stream_source_recovery_pending`].
    pub(crate) unsafe fn source_recovery_pending(
        &self,
        link: u64,
        pending: *mut u32,
    ) -> Result<(), Error> {
        let source = self.source(link)?;
        let pending = writable(pending)?;
        // SAFETY: `pending` is writable for the call, as the caller promises, and was checked.
        unsafe { pending.write(u32::from(source.recovery_pending())) };
        Ok(())
    }

    /// Sends the picture that `begin` announced. The frame is given up on if anything is wrong.
    ///
    /// # Safety
    ///
    /// As for [`baton_stream_source_sent`].
    pub(crate) unsafe fn source_sent(
        &self,
        link: u64,
        picture: *const BatonPicture,
        outcome: *mut u32,
        number: *mut u64,
    ) -> Result<(), Error> {
        let source = self.source(link)?;
        // SAFETY: the caller's promise about the pointers is this function's own.
        let sent = unsafe { send_picture(&source, picture, outcome, number) };
        if sent.is_err() {
            source.cancel();
        }
        sent
    }

    /// The window changed at `capture_time`.
    pub(crate) fn source_changed(&self, link: u64, capture_time: u64) -> Result<(), Error> {
        self.source(link)?.changed(capture_time);
        Ok(())
    }

    /// The size of the picture changed.
    ///
    /// # Safety
    ///
    /// As for [`baton_stream_source_new_epoch`].
    pub(crate) unsafe fn source_new_epoch(
        &self,
        link: u64,
        format: *const BatonFormat,
    ) -> Result<(), Error> {
        let source = self.source(link)?;
        // SAFETY: the caller's promise about `format` is this function's own.
        let format = unsafe { read_struct(format, size_of::<BatonFormat>()) }?;
        source.new_epoch(&Chosen {
            codec: format.codec,
            width: format.width,
            height: format.height,
            fps: format.fps,
        })
    }

    /// Stops the pictures for a while.
    pub(crate) fn source_pause(&self, link: u64, reason: u32) -> Result<(), Error> {
        self.source(link)?.pause(reason)
    }

    /// The pictures go on.
    pub(crate) fn source_resume(&self, link: u64) -> Result<(), Error> {
        self.source(link)?.resume();
        Ok(())
    }

    /// Ends the session.
    ///
    /// # Safety
    ///
    /// As for [`baton_stream_source_end`].
    pub(crate) unsafe fn source_end(
        &self,
        link: u64,
        sentence: *const BatonBytes,
    ) -> Result<(), Error> {
        let source = self.source(link)?;
        // SAFETY: the caller's promise about `sentence` is this function's own.
        let sentence = unsafe { read_sentence(sentence) }?;
        source.end(sentence);
        Ok(())
    }
}

/// A pointer that is written through: not null and aligned. Nothing is written through it here.
fn writable<T>(pointer: *mut T) -> Result<NonNull<T>, Error> {
    NonNull::new(pointer)
        .filter(|pointer| pointer.as_ptr().is_aligned())
        .ok_or(Error::Argument)
}

/// Copies a struct that the platform passed, if it is whole: the pointer is not null and is
/// aligned, and the `u32` at its start, which says how many bytes the struct has, holds the first
/// `mandatory` bytes of it, which the caller chooses. A struct that ends after those but before
/// the end of what this version knows is from an older platform, and what it lacks reads as zero,
/// also a field that its size cuts in two. A longer struct is accepted, and what follows the part
/// that this version knows is not read.
///
/// # Safety
///
/// `pointer` is either null, or points to a struct that begins with a `u32` that is its size in
/// bytes, and is that long, and that nothing writes to during this call. All zero bytes are a
/// valid `T`: it holds numbers, pointers and optional functions, and nothing else.
unsafe fn read_struct<T: Copy>(pointer: *const T, mandatory: usize) -> Result<T, Error> {
    if pointer.is_null() || !pointer.is_aligned() {
        return Err(Error::Argument);
    }
    // SAFETY: not null and aligned, and the caller promises that a `u32` is first.
    let size = unsafe { pointer.cast::<u32>().read() };
    let Ok(size) = usize::try_from(size) else {
        return Err(Error::Argument);
    };
    if size < mandatory {
        return Err(Error::Argument);
    }
    // A size that ends in the middle of a field is not copied to the end: half of a number is not
    // a number. No field crosses a multiple of the alignment of the struct, so a copy that ends
    // at one never cuts a field in two.
    let copied = size.min(size_of::<T>()) & !(align_of::<T>() - 1);
    let mut whole = MaybeUninit::<T>::zeroed();
    // SAFETY: the struct is at least `size` bytes long, as the caller promises, and the bytes that
    // are copied are the first of them, no more than `T` has; they land in a `T` that was zeroed
    // first. The bytes after `T` belong to a newer version and are not read.
    unsafe {
        ptr::copy_nonoverlapping(
            pointer.cast::<u8>(),
            whole.as_mut_ptr().cast::<u8>(),
            copied,
        );
    }
    // SAFETY: the caller promises that all zero bytes are a valid `T`, and so is a zeroed `T`
    // that has had the platform's bytes written over its start.
    Ok(unsafe { whole.assume_init() })
}

/// The bytes that `bytes` describes, up to `most` of them: none when the length is zero, whatever
/// the pointer is, and an error for a null pointer with a length.
///
/// # Safety
///
/// If `bytes.len` is not zero, `bytes.ptr` points to that many bytes that can be read during the
/// call and that nothing writes to meanwhile. The slice must not be used past the call.
unsafe fn read_bytes<'a>(bytes: &BatonBytes, most: usize) -> Result<&'a [u8], Error> {
    if bytes.len == 0 {
        return Ok(&[]);
    }
    if bytes.ptr.is_null() {
        return Err(Error::Argument);
    }
    // SAFETY: not null, and the caller promises that `len` bytes can be read; fewer are taken.
    Ok(unsafe { slice::from_raw_parts(bytes.ptr, bytes.len.min(most)) })
}

/// The text in `bytes`, as far as it is text: it ends at a zero byte (a platform written in C may
/// count the one that ends its string), what is not UTF-8 is replaced, and it is cut to at most
/// `most` bytes at the end of a character.
fn text(bytes: &[u8], most: usize) -> String {
    let bytes = bytes.split(|byte| *byte == 0).next().unwrap_or(bytes);
    let text = String::from_utf8_lossy(bytes);
    let mut end = text.len().min(most);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// The sentence that `pointer` leads to, which says what went wrong in words for the person, or
/// `None` if there is none: the pointer is null, or the bytes are empty or all white space.
///
/// # Safety
///
/// `pointer` is either null, or points to a `BatonBytes` that can be read during the call, and
/// the bytes that it describes can be read too, if there are any, and nothing writes to either
/// meanwhile.
unsafe fn read_sentence(pointer: *const BatonBytes) -> Result<Option<String>, Error> {
    if pointer.is_null() {
        return Ok(None);
    }
    if !pointer.is_aligned() {
        return Err(Error::Argument);
    }
    // SAFETY: not null and aligned, and the caller promises that it can be read.
    let bytes = unsafe { pointer.read() };
    // SAFETY: the caller's promise about the bytes is this function's own.
    let bytes = unsafe { read_bytes(&bytes, MESSAGE_MAX as usize) }?;
    let sentence = text(bytes, MESSAGE_MAX as usize);
    let sentence = sentence.trim();
    Ok((!sentence.is_empty()).then(|| sentence.to_owned()))
}

/// What the platform answered to a pick: a window that the person chose, a sentence that says why
/// the picker failed, or that the person gave up.
///
/// # Safety
///
/// As for [`baton_stream_pick_done`].
unsafe fn read_answer(
    picked: *const BatonPicked,
    failure: *const BatonBytes,
) -> Result<PickAnswer, Error> {
    // SAFETY: the caller's promise about `failure` is this function's own.
    let failure = unsafe { read_sentence(failure) }?;
    if picked.is_null() {
        return Ok(failure.map_or(PickAnswer::GaveUp, PickAnswer::Failed));
    }
    if failure.is_some() {
        // A window and a failure in one answer: the platform does not know what it is saying.
        return Err(Error::Argument);
    }
    // SAFETY: the caller's promise about `picked` is this function's own.
    let picked = unsafe { read_struct(picked, size_of::<BatonPicked>()) }?;
    let context = NonNull::new(picked.context).ok_or(Error::Argument)?;
    if picked.width == 0 || picked.height == 0 {
        return Err(Error::Argument);
    }
    // A platform that is older than the field wrote zero, which is a window. A value that is
    // neither is a kind of thing that this version cannot offer, and so it is refused.
    let display = match picked.kind {
        PICKED_WINDOW => false,
        PICKED_DISPLAY => true,
        _ => return Err(Error::Argument),
    };
    // SAFETY: the caller's promise about the bytes of the title and the name is this function's
    // own, since they are described by the struct that it vouches for.
    let (title, app) = unsafe {
        (
            read_bytes(&picked.title, MAX_STREAM_TITLE_BYTES)?,
            read_bytes(&picked.app, MAX_NAME_BYTES)?,
        )
    };
    Ok(PickAnswer::Chosen(Given {
        context,
        display,
        title: text(title, MAX_STREAM_TITLE_BYTES),
        app: text(app, MAX_NAME_BYTES),
        width: picked.width.min(MAX_WINDOW_EDGE),
        height: picked.height.min(MAX_WINDOW_EDGE),
        // A window that the person picked is not one that a screen was kept private for.
        private: false,
    }))
}

/// Reads the picture that the platform made, sends it, and tells the platform what became of it.
///
/// # Safety
///
/// As for [`baton_stream_source_sent`].
unsafe fn send_picture(
    source: &SourceLink,
    picture: *const BatonPicture,
    outcome: *mut u32,
    number: *mut u64,
) -> Result<(), Error> {
    let (outcome, number) = (writable(outcome)?, writable(number)?);
    // SAFETY: the caller's promise about `picture` is this function's own. A struct that ends
    // before `geometry` is from a platform that does not carry input: the geometry is zero.
    let picture = unsafe { read_struct(picture, offset_of!(BatonPicture, geometry)) }?;
    let sent = if picture.sample.len > MAX_PICTURE_BYTES {
        // Not read: a length that large is not a picture, and what is behind it is the
        // platform's business.
        source.too_large()
    } else {
        let length_size = match picture.length_size {
            1 | 2 | 4 => picture.length_size as usize,
            _ => return Err(Error::Argument),
        };
        // SAFETY: the caller's promise about the sample and the parameter sets is this
        // function's own, since the struct that describes them is the one it vouches for.
        let (sample, sets) = unsafe {
            (
                read_bytes(&picture.sample, MAX_PICTURE_BYTES)?,
                read_sets(&picture)?,
            )
        };
        source.sent(&Made {
            referenced: picture.flags & FRAME_REFERENCED != 0,
            capture_time: picture.capture_time,
            length_size,
            sample,
            sets,
            geometry: geometry_of(&picture.geometry),
        })?
    };
    // SAFETY: both are writable for the call, as the caller promises, and were checked.
    unsafe {
        outcome.write(sent.code());
        number.write(sent.number());
    }
    Ok(())
}

/// The parameter sets of a picture, each as the bytes it describes.
///
/// # Safety
///
/// If `picture.set_count` is not zero, `picture.sets` points to that many `BatonBytes` that can be
/// read during the call, and each describes bytes that can be read too, and nothing writes to
/// either meanwhile. The slices must not be used past the call.
unsafe fn read_sets<'a>(picture: &BatonPicture) -> Result<Vec<&'a [u8]>, Error> {
    if picture.set_count == 0 {
        return Ok(Vec::new());
    }
    if picture.set_count > SETS_MAX as usize || picture.sets.is_null() || !picture.sets.is_aligned()
    {
        return Err(Error::Argument);
    }
    // SAFETY: not null, aligned, and the caller promises that `set_count` entries can be read.
    let sets = unsafe { slice::from_raw_parts(picture.sets, picture.set_count) };
    sets.iter()
        .map(|set| {
            if set.len > SET_BYTES_MAX as usize {
                return Err(Error::Argument);
            }
            // SAFETY: the caller's promise about the bytes of a set is this function's own.
            unsafe { read_bytes(set, SET_BYTES_MAX as usize) }
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::{
        ptr,
        sync::{Arc, Mutex},
    };

    use baton_app::windows::ScreenLink;

    use super::*;
    use crate::{
        abi::{
            BatonGeometry, BatonRect, E_ARGUMENT, E_PLATFORM, E_REENTRANT, E_REGISTERED,
            E_UNKNOWN_LINK, OK, size_of_u32,
        },
        fake::{Call, Fake, Watchdog},
        fake_sharer::FakeSharer,
        registered_windows,
    };

    #[test]
    fn a_panic_is_answered_with_its_number_and_not_let_through() {
        assert_eq!(answer(|| panic!("a bug")), Error::Panic.code());
        assert_eq!(answer(|| Ok(())), 0);
        assert_eq!(answer(|| Err(Error::Argument)), Error::Argument.code());
    }

    /// The one test that registers a viewer in the registry of the process, since that can be done
    /// once for as long as the process lives. Every other test makes a registry of its own.
    #[test]
    fn the_functions_of_the_interface_work_on_the_registry_of_the_process() {
        assert_eq!(baton_stream_abi_version(), ABI_VERSION);
        assert!(global().viewer().is_none());
        assert!(format!("{:?}", registered_windows()).contains("shows: false"));

        // Whatever the platform passes, what it gets back is a number.
        // SAFETY: a null vtable is one of the arguments that the function is made for.
        assert_eq!(
            unsafe { baton_stream_register_viewer(ptr::null()) },
            E_ARGUMENT
        );
        let fake = Fake::new();
        let newer = BatonViewerVTable {
            version: ABI_VERSION + 1,
            ..fake.vtable()
        };
        // SAFETY: a vtable that is whole, and that nothing writes to meanwhile.
        assert_eq!(
            unsafe { baton_stream_register_viewer(&raw const newer) },
            E_ARGUMENT
        );
        fake.fail_limits(-7);
        // SAFETY: as above.
        assert_eq!(
            unsafe { baton_stream_register_viewer(&fake.vtable()) },
            E_PLATFORM
        );
        fake.fail_limits(OK);
        assert!(
            global().viewer().is_none(),
            "a registration that failed left a viewer"
        );

        // The platform calls the interface from inside the call that is registering it, which
        // would wait for itself if it were let in.
        let inside = Arc::new(Mutex::new(Vec::new()));
        let platform = Arc::downgrade(&fake);
        let heard = Arc::clone(&inside);
        fake.on_call(move |call| {
            if *call == Call::Limits {
                let nested = platform.upgrade().unwrap().vtable();
                // A call that was let in would wait for the registration that is being made.
                let _watch = Watchdog::start("a call into the interface from inside a call");
                // SAFETY: as above.
                let registering = unsafe { baton_stream_register_viewer(&raw const nested) };
                heard
                    .lock()
                    .unwrap()
                    .push((registering, baton_stream_link_decode_failed(1)));
            }
        });
        let vtable = fake.vtable();
        // SAFETY: as above.
        assert_eq!(
            unsafe { baton_stream_register_viewer(&raw const vtable) },
            OK
        );
        assert_eq!(*inside.lock().unwrap(), [(E_REENTRANT, E_REENTRANT)]);
        assert!(global().viewer().is_some());
        assert!(format!("{:?}", registered_windows()).contains("shows: true"));

        // A viewer is registered once, whatever registers the second.
        // SAFETY: as above.
        assert_eq!(
            unsafe { baton_stream_register_viewer(&raw const vtable) },
            E_REGISTERED
        );
        // Limits were asked of the platform for the registration that failed and the one that
        // worked, and for none of the others.
        assert_eq!(fake.count(|call| *call == Call::Limits), 2);

        // A link is open from the moment `open` gives it to the platform until its screen is
        // released, and a number that is not open is told so.
        assert_eq!(baton_stream_link_decode_failed(0), E_UNKNOWN_LINK);
        assert_eq!(baton_stream_link_decode_failed(u64::MAX), E_UNKNOWN_LINK);
        let link = global().links.add(ScreenLink::default());
        assert_eq!(baton_stream_link_decode_failed(link), OK);
        global().links.remove(link);
        assert_eq!(baton_stream_link_decode_failed(link), E_UNKNOWN_LINK);
        assert!(fake.complaints().is_empty(), "{:?}", fake.complaints());
    }

    /// The one test that registers a sharer in the registry of the process, since that can be done
    /// once for as long as the process lives.
    #[test]
    fn the_functions_of_the_interface_for_sharing_work_on_the_registry_of_the_process() {
        assert!(global().sharer().is_none());
        assert!(format!("{:?}", registered_windows()).contains("shares: false"));

        // Before anything is registered, and for any link, what the platform gets is a number.
        let (mut micros, mut small, mut number) = (0_u64, 0_u32, 0_u64);
        let mut everything = |link: u64| {
            // SAFETY: null pointers are among the arguments that the functions are made for, and
            // the others point to values that live through the calls.
            unsafe {
                [
                    baton_stream_source_clock(link, &raw mut micros),
                    baton_stream_source_begin(link, &raw mut small),
                    baton_stream_source_cancel(link),
                    baton_stream_source_recovery_pending(link, &raw mut small),
                    baton_stream_source_sent(link, ptr::null(), &raw mut small, &raw mut number),
                    baton_stream_source_changed(link, 1),
                    baton_stream_source_new_epoch(link, ptr::null()),
                    baton_stream_source_pause(link, 0),
                    baton_stream_source_resume(link),
                    baton_stream_source_end(link, ptr::null()),
                ]
            }
        };
        for link in [0, 1, 99, u64::MAX] {
            assert_eq!(everything(link), [E_UNKNOWN_LINK; 10], "link {link}");
        }
        assert_eq!(
            // SAFETY: both null is one of the answers that the function is made for.
            unsafe { baton_stream_pick_done(1, ptr::null(), ptr::null()) },
            E_UNKNOWN_LINK
        );

        // Whatever the platform passes to register, what it gets back is a number.
        // SAFETY: a null vtable is one of the arguments that the function is made for.
        assert_eq!(
            unsafe { baton_stream_register_sharer(ptr::null()) },
            E_ARGUMENT
        );
        let platform = FakeSharer::new();
        let newer = BatonSharerVTable {
            version: ABI_VERSION + 1,
            ..platform.vtable()
        };
        // SAFETY: a vtable that is whole, and that nothing writes to meanwhile.
        assert_eq!(
            unsafe { baton_stream_register_sharer(&raw const newer) },
            E_ARGUMENT
        );
        let incomplete = BatonSharerVTable {
            release: None,
            ..platform.vtable()
        };
        // SAFETY: as above.
        assert_eq!(
            unsafe { baton_stream_register_sharer(&raw const incomplete) },
            E_ARGUMENT
        );
        assert!(global().sharer().is_none());

        let table = platform.vtable();
        // SAFETY: as above.
        assert_eq!(
            unsafe { baton_stream_register_sharer(&raw const table) },
            OK
        );
        // SAFETY: as above.
        assert_eq!(
            unsafe { baton_stream_register_sharer(&raw const table) },
            E_REGISTERED
        );
        assert!(format!("{:?}", registered_windows()).contains("shares: true"));

        // A person picks a window through what the app is given, and the platform answers with the
        // functions of the interface.
        let sharer = global().sharer().expect("the sharer was registered");
        let mut pick = sharer.pick();
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(pick.as_mut().poll(&mut context).is_pending());
        assert_eq!(platform.last_request(), Some(1));
        assert_eq!(global().waiting_picks(), 1);
        // SAFETY: both null is one of the answers that the function is made for.
        assert_eq!(
            unsafe { baton_stream_pick_done(1, ptr::null(), ptr::null()) },
            OK
        );
        assert_eq!(
            // SAFETY: as above.
            unsafe { baton_stream_pick_done(1, ptr::null(), ptr::null()) },
            E_UNKNOWN_LINK
        );
        assert!(matches!(
            pick.as_mut().poll(&mut context),
            std::task::Poll::Ready(Ok(None))
        ));
        assert_eq!(global().waiting_picks(), 0);
        assert!(
            platform.complaints().is_empty(),
            "{:?}",
            platform.complaints()
        );
    }

    #[test]
    fn text_ends_at_a_zero_byte_replaces_what_is_not_utf8_and_is_cut_at_a_character() {
        assert_eq!(text(b"abc", 10), "abc");
        assert_eq!(text(b"ab\0cd", 10), "ab");
        assert_eq!(text(b"\0abc", 10), "");
        assert_eq!(text(b"a\xffb", 10), "a\u{fffd}b");
        assert_eq!(text(b"abc", 0), "");
        // "é" is two bytes, so a cut in the middle of it goes before it.
        assert_eq!(text("aé".as_bytes(), 3), "aé");
        assert_eq!(text("aé".as_bytes(), 2), "a");
        assert_eq!(text("aé".as_bytes(), 1), "a");
        assert_eq!(text("é".as_bytes(), 1), "");
        // The mark that stands for a byte that is not text is three bytes long.
        assert_eq!(text(b"\xff\xff", 4), "\u{fffd}");
        assert_eq!(text(b"\xff\xff", 6), "\u{fffd}\u{fffd}");
    }

    #[test]
    fn bytes_are_read_when_there_are_some_and_only_as_far_as_they_may_be() {
        // None: the pointer is not looked at, whatever it is.
        for ptr in [ptr::null(), 7 as *const u8, usize::MAX as *const u8] {
            let none = BatonBytes { ptr, len: 0 };
            // SAFETY: no bytes are described, so none are read.
            assert_eq!(unsafe { read_bytes(&none, 10) }, Ok(&[][..]));
        }
        // Some, and nowhere to read them from.
        let nowhere = BatonBytes {
            ptr: ptr::null(),
            len: 1,
        };
        // SAFETY: a null pointer is refused before it is read.
        assert_eq!(unsafe { read_bytes(&nowhere, 10) }, Err(Error::Argument));

        let data: Vec<u8> = (0..=255).cycle().take(1000).collect();
        let bytes = BatonBytes::of(&data);
        for (most, read) in [(0, 0), (10, 10), (1000, 1000), (5000, 1000)] {
            // SAFETY: `data` is 1000 bytes long, and lives through the call.
            assert_eq!(
                unsafe { read_bytes(&bytes, most) },
                Ok(&data[..read]),
                "at most {most}"
            );
        }
    }

    #[test]
    fn a_sentence_is_what_the_platform_wrote_as_far_as_it_says_something() {
        // SAFETY: null is one of the arguments that the function is made for.
        assert_eq!(unsafe { read_sentence(ptr::null()) }, Ok(None));
        let read = |bytes: &[u8]| {
            let bytes = BatonBytes::of(bytes);
            // SAFETY: a `BatonBytes` that lives through the call, with bytes that do too.
            unsafe { read_sentence(&raw const bytes) }
        };
        for nothing in [&b""[..], b"   ", b"\n\t ", b"\0abc", b" \0 x"] {
            assert_eq!(read(nothing), Ok(None), "{nothing:?}");
        }
        assert_eq!(
            read(b"Capture failed."),
            Ok(Some("Capture failed.".to_owned()))
        );
        assert_eq!(
            read(b"  Capture failed.\n"),
            Ok(Some("Capture failed.".to_owned()))
        );
        assert_eq!(read(b"Capture\0failed."), Ok(Some("Capture".to_owned())));
        // At most 256 bytes, and not in the middle of a character.
        assert_eq!(read(&[b'x'; 1000]), Ok(Some("x".repeat(256))));
        let cut = ["x".repeat(255).as_bytes(), "é".as_bytes()].concat();
        assert_eq!(read(&cut), Ok(Some("x".repeat(255))));

        let nowhere = BatonBytes {
            ptr: ptr::null(),
            len: 3,
        };
        // SAFETY: a null pointer for bytes is refused before it is read.
        assert_eq!(
            unsafe { read_sentence(&raw const nowhere) },
            Err(Error::Argument)
        );
        // A pointer that is not aligned is refused without being read.
        let buffer = [0_u64; 4];
        // SAFETY: the pointer is not read: it is refused for the way it is aligned.
        let unaligned = unsafe { read_sentence(buffer.as_ptr().cast::<u8>().add(1).cast()) };
        assert_eq!(unaligned, Err(Error::Argument));
    }

    #[test]
    fn a_struct_is_copied_only_if_it_states_a_size_that_holds_what_this_version_knows() {
        let whole = size_of_u32::<BatonFormat>();
        let format = |size| BatonFormat {
            size,
            codec: 1,
            width: 2,
            height: 3,
            fps: 4,
            reserved: 0,
        };
        let read = |format: &BatonFormat| {
            // SAFETY: a struct that begins with its size, and lives through the call.
            unsafe { read_struct(ptr::from_ref(format), size_of::<BatonFormat>()) }
                .map(|read| read.width)
        };
        assert_eq!(read(&format(whole)), Ok(2));
        // A longer struct is a newer one: what follows the part that is known is not read.
        assert_eq!(read(&format(whole + 8)), Ok(2));
        for size in [0, 4, whole - 1] {
            assert_eq!(read(&format(size)), Err(Error::Argument), "size {size}");
        }
        // SAFETY: null is one of the arguments that the function is made for.
        assert_eq!(
            unsafe { read_struct::<BatonFormat>(ptr::null(), size_of::<BatonFormat>()) }
                .map(|_| ()),
            Err(Error::Argument)
        );
        // A pointer that is not aligned is refused without being read, also when a whole struct is
        // at the address: a buffer of zeros would be refused for its size.
        let mut buffer = [0_u64; 8];
        // SAFETY: the buffer has room for a struct from its second byte on.
        #[allow(clippy::cast_ptr_alignment)] // the pointer is not aligned, which is the test
        let at = unsafe {
            buffer
                .as_mut_ptr()
                .cast::<u8>()
                .add(1)
                .cast::<BatonFormat>()
        };
        // SAFETY: `at` is inside the buffer, with room for the struct, and is written unaligned.
        unsafe { at.write_unaligned(format(whole)) };
        // SAFETY: the pointer is not read: it is refused for the way it is aligned.
        let unaligned = unsafe { read_struct::<BatonFormat>(at, size_of::<BatonFormat>()) };
        assert_eq!(unaligned.map(|_| ()), Err(Error::Argument));
    }

    #[test]
    fn what_a_struct_lacks_reads_as_zero_and_a_field_that_its_size_cuts_is_not_read_at_all() {
        let whole = BatonPicture {
            size: size_of_u32::<BatonPicture>(),
            flags: FRAME_REFERENCED,
            capture_time: 7,
            length_size: 4,
            reserved: 0,
            sample: BatonBytes {
                ptr: ptr::null(),
                len: 0,
            },
            sets: ptr::null(),
            set_count: 0,
            geometry: BatonGeometry {
                window_width: 400.0,
                window_height: 300.0,
                picture_width: 64,
                picture_height: 48,
                content: BatonRect {
                    x: 1.0,
                    y: 2.0,
                    width: 3.0,
                    height: 4.0,
                },
            },
        };
        let geometry_at = offset_of!(BatonPicture, geometry);
        // What the platform says its struct is, with all of the memory there all the same.
        let read = |size: usize| {
            let picture = BatonPicture {
                size: u32::try_from(size).unwrap(),
                ..whole
            };
            // SAFETY: a struct that begins with its size, and lives through the call.
            unsafe { read_struct(ptr::from_ref(&picture), geometry_at) }.unwrap()
        };
        assert_eq!(read(size_of::<BatonPicture>()).geometry, whole.geometry);
        // A longer struct is a newer one, and what follows is not read.
        assert_eq!(
            read(size_of::<BatonPicture>() + 64).geometry,
            whole.geometry
        );
        // A struct from before the layout was carried has none.
        assert_eq!(read(geometry_at).geometry, BatonGeometry::default());
        assert_eq!(read(geometry_at).capture_time, 7);
        // A struct that ends after some of the fields has those, and the rest are zero.
        let some = read(geometry_at + 16).geometry;
        assert_eq!((some.window_width, some.window_height), (400.0, 300.0));
        assert_eq!((some.picture_width, some.picture_height), (0, 0));
        assert_eq!(some.content, BatonRect::default());
        let more = read(geometry_at + 24).geometry;
        assert_eq!((more.picture_width, more.picture_height), (64, 48));
        assert_eq!(more.content, BatonRect::default());
        // A size that cuts a field in two gives none of the field, and not half of it: no number
        // is made of the first bytes of another.
        for cut in 1..8 {
            let first = read(geometry_at + cut).geometry;
            assert_eq!(first, BatonGeometry::default(), "cut {cut}");
            let second = read(geometry_at + 8 + cut).geometry;
            assert_eq!(second.window_width, 400.0, "cut {cut}");
            assert_eq!(second.window_height, 0.0, "cut {cut}");
        }
        // A size that is less than what every version has is refused.
        let short = BatonPicture {
            size: u32::try_from(geometry_at - 1).unwrap(),
            ..whole
        };
        // SAFETY: a struct that begins with its size, and lives through the call.
        let refused = unsafe { read_struct(ptr::from_ref(&short), geometry_at) };
        assert_eq!(refused.map(|_| ()), Err(Error::Argument));
    }

    #[test]
    fn a_pointer_that_is_written_through_must_be_there_and_aligned() {
        assert!(writable::<u64>(ptr::null_mut()).is_err());
        let mut buffer = [0_u64; 2];
        assert!(writable(buffer.as_mut_ptr()).is_ok());
        // SAFETY: only an address is made; nothing is written through it.
        #[allow(clippy::cast_ptr_alignment)] // the pointer is not aligned, which is the test
        let unaligned = unsafe { buffer.as_mut_ptr().cast::<u8>().add(1).cast::<u64>() };
        assert!(writable(unaligned).is_err());
        // One byte wide, any address is aligned.
        assert!(writable(unaligned.cast::<u8>()).is_ok());
    }

    #[test]
    fn a_link_that_is_not_open_is_refused_before_anything_else_is_looked_at() {
        let registry = Registry::new();
        let nowhere = BatonBytes {
            ptr: ptr::null(),
            len: 3,
        };
        // SAFETY: a sentence that is described and cannot be read is refused, and it is not even
        // looked at when no session is open under the link.
        let ended = unsafe { registry.source_end(99, &raw const nowhere) };
        assert_eq!(ended, Err(Error::UnknownLink));
    }

    #[test]
    fn parameter_sets_are_read_as_far_as_the_header_allows() {
        let picture = |sets: *const BatonBytes, set_count: usize| BatonPicture {
            size: size_of_u32::<BatonPicture>(),
            flags: 0,
            capture_time: 0,
            length_size: 4,
            reserved: 0,
            sample: BatonBytes::of(b""),
            sets,
            set_count,
            geometry: BatonGeometry::default(),
        };
        let read = |picture: &BatonPicture| {
            // SAFETY: the sets that it describes live through the call.
            unsafe { read_sets(picture) }
        };
        // None: the pointer is not looked at, whatever it is.
        for sets in [ptr::null(), ptr::dangling::<BatonBytes>()] {
            assert_eq!(read(&picture(sets, 0)), Ok(Vec::new()));
        }
        // Some, and nowhere to read them from, or not aligned.
        assert_eq!(read(&picture(ptr::null(), 1)), Err(Error::Argument));
        let buffer = [0_u64; 8];
        // SAFETY: only an address is made; it is refused for the way it is aligned.
        #[allow(clippy::cast_ptr_alignment)] // the pointer is not aligned, which is the test
        let unaligned = unsafe { buffer.as_ptr().cast::<u8>().add(1).cast::<BatonBytes>() };
        assert_eq!(read(&picture(unaligned, 1)), Err(Error::Argument));

        let (sequence, picture_set, empty) = (
            b"\x67\x42".as_slice(),
            b"\x68\xce".as_slice(),
            b"".as_slice(),
        );
        let sets = [
            BatonBytes::of(sequence),
            BatonBytes::of(picture_set),
            BatonBytes::of(empty),
        ];
        assert_eq!(
            read(&picture(sets.as_ptr(), 3)),
            Ok(vec![sequence, picture_set, empty])
        );

        // The most that the header allows is read, and one more is not.
        let big = vec![9_u8; SET_BYTES_MAX as usize];
        let most: Vec<BatonBytes> = (0..SETS_MAX).map(|_| BatonBytes::of(&big)).collect();
        let read_most = read(&picture(most.as_ptr(), most.len())).unwrap();
        assert_eq!(read_most.len(), SETS_MAX as usize);
        assert!(read_most.iter().all(|set| *set == big.as_slice()));
        let too_many: Vec<BatonBytes> = (0..=SETS_MAX).map(|_| BatonBytes::of(&big)).collect();
        assert_eq!(
            read(&picture(too_many.as_ptr(), too_many.len())),
            Err(Error::Argument)
        );
        // A set that is too long is refused without being read (the pointer leads nowhere).
        let long = [BatonBytes {
            ptr: 8 as *const u8,
            len: SET_BYTES_MAX as usize + 1,
        }];
        assert_eq!(read(&picture(long.as_ptr(), 1)), Err(Error::Argument));
        // And so is a set with a length and no pointer.
        let nowhere = [BatonBytes {
            ptr: ptr::null(),
            len: 2,
        }];
        assert_eq!(read(&picture(nowhere.as_ptr(), 1)), Err(Error::Argument));
    }
}
