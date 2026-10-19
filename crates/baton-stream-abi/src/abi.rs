// SPDX-License-Identifier: Apache-2.0

//! The structs, functions and numbers of `include/baton_stream_abi.h`, in the layout that the
//! header has. The header says what each of them means; the comments here only say where Rust
//! needs more than C does. `tests/header.rs` holds every size, offset and number below to the
//! header.

use std::ffi::c_void;

/// The version of the interface. It changes only when something is taken away or moved.
pub const ABI_VERSION: u32 = 1;

/// What a function of the interface returns when it worked, and what a call of the vtable returns
/// when it worked.
pub const OK: i32 = 0;
/// A null pointer, a wrong version or size, a function that is missing, or a wrong value.
pub const E_ARGUMENT: i32 = -1;
/// A viewer was registered already.
pub const E_REGISTERED: i32 = -2;
/// A call of the vtable failed.
pub const E_PLATFORM: i32 = -3;
/// The link is not open.
pub const E_UNKNOWN_LINK: i32 = -4;
/// Called from inside a call of the vtable, on the thread that the call is made on.
pub const E_REENTRANT: i32 = -5;
/// The Rust side went wrong inside the call.
pub const E_PANIC: i32 = -6;

/// The codec bit of H.264 in Annex B form on the wire.
pub const CODEC_H264: u32 = 1;

/// A key frame.
pub const FRAME_KEY: u32 = 1;
/// A recovery frame.
pub const FRAME_RECOVERY: u32 = 2;
/// Another frame may refer to this one.
pub const FRAME_REFERENCED: u32 = 4;

/// The picture that follows.
pub const SOURCE_CONFIG: u32 = 1;
/// Pictures stop for a while.
pub const SOURCE_PAUSE: u32 = 2;
/// Pictures go on.
pub const SOURCE_RESUME: u32 = 3;
/// Events were dropped.
pub const SOURCE_NOTICE: u32 = 4;

/// A reason to pause that this version does not know.
pub const PAUSE_OTHER: u32 = 0;
/// The Mac is locked, or its display is off.
pub const PAUSE_LOCKED: u32 = 1;
/// The shared window is minimised or not on screen.
pub const PAUSE_HIDDEN: u32 = 2;

/// A reason to drop events that this version does not know.
pub const DROPPED_OTHER: u32 = 0;
/// The event has a number or a length that is not allowed.
pub const DROPPED_MALFORMED: u32 = 1;
/// Control is off.
pub const DROPPED_CONTROL_OFF: u32 = 2;
/// The session is paused.
pub const DROPPED_PAUSED: u32 = 3;
/// The Mac is locked.
pub const DROPPED_LOCKED: u32 = 4;
/// The shared window is not on screen.
pub const DROPPED_NOT_ON_SCREEN: u32 = 5;
/// More events in a second than allowed.
pub const DROPPED_TOO_FAST: u32 = 6;
/// The frame that the event names is not one whose geometry is kept.
pub const DROPPED_FRAME_UNKNOWN: u32 = 7;
/// The picture went out of date too long ago.
pub const DROPPED_PICTURE_OUT_OF_DATE: u32 = 8;
/// The window changed its size since that frame.
pub const DROPPED_WINDOW_RESIZED: u32 = 9;
/// The point is outside the window.
pub const DROPPED_OUTSIDE_WINDOW: u32 = 10;
/// Another window is in front of the point.
pub const DROPPED_COVERED: u32 = 11;
/// The shared window does not have the keyboard.
pub const DROPPED_NO_KEYBOARD: u32 = 12;
/// A window of the same application is in front of the shared window.
pub const DROPPED_APP_WINDOW_IN_FRONT: u32 = 13;
/// More keys are held down than a person has fingers.
pub const DROPPED_TOO_MANY_HELD: u32 = 14;
/// A release of something this session did not press.
pub const DROPPED_NOT_PRESSED: u32 = 15;

/// No limit.
pub const THERMAL_NOMINAL: u32 = 0;
/// Slightly warm.
pub const THERMAL_FAIR: u32 = 1;
/// Hot.
pub const THERMAL_SERIOUS: u32 = 2;
/// As hot as it gets.
pub const THERMAL_CRITICAL: u32 = 3;
/// Any other state.
pub const THERMAL_OTHER: u32 = 4;

/// The most bytes of the sentence that explains a failure.
pub const MESSAGE_MAX: u32 = 256;

/// The size of a struct for its `size` field. The structs of the interface are a few dozen bytes.
#[allow(clippy::cast_possible_truncation)]
pub(crate) const fn size_of_u32<T>() -> u32 {
    let size = size_of::<T>();
    assert!(size <= u32::MAX as usize);
    size as u32
}

/// Bytes in memory that the other side owns.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BatonBytes {
    /// The first byte. Not to be read when `len` is zero.
    pub ptr: *const u8,
    /// How many bytes.
    pub len: usize,
}

impl BatonBytes {
    /// The bytes of a slice, for as long as the slice lives.
    pub(crate) fn of(bytes: &[u8]) -> Self {
        Self {
            ptr: bytes.as_ptr(),
            len: bytes.len(),
        }
    }
}

/// What the device can show.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BatonLimits {
    /// The bytes the platform may write.
    pub size: u32,
    /// The widest picture in pixels.
    pub max_width: u32,
    /// The highest picture in pixels.
    pub max_height: u32,
    /// The most frames a second.
    pub max_fps: u32,
    /// The codecs the decoder takes, as bits.
    pub codecs: u32,
    /// Zero.
    pub reserved: u32,
}

/// The window that the person said yes to.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BatonWindow {
    /// The bytes of this struct.
    pub size: u32,
    /// The width of the window in pixels.
    pub width: u32,
    /// The height of the window in pixels.
    pub height: u32,
    /// 1 if this device may control the window.
    pub control: u32,
    /// The title. What it is called is not for a log.
    pub title: BatonBytes,
    /// The name of the application.
    pub app: BatonBytes,
}

/// The screen that `open` makes.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BatonScreen {
    /// The bytes the platform may write.
    pub size: u32,
    /// Zero.
    pub reserved: u32,
    /// The platform's own pointer for the screen.
    pub context: *mut c_void,
    /// The number the user interface draws the screen with.
    pub id: i64,
}

/// Room for the sentence that says why `open` failed.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BatonMessage {
    /// The bytes of this struct.
    pub size: u32,
    /// The bytes the platform may write to `text`.
    pub cap: u32,
    /// The bytes it wrote.
    pub len: u32,
    /// Zero.
    pub reserved: u32,
    /// Where to write.
    pub text: *mut u8,
}

/// A frame to decode.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BatonFrame {
    /// The bytes of this struct.
    pub size: u32,
    /// `FRAME_` bits.
    pub flags: u32,
    /// Counts the frames of the session from 0.
    pub number: u64,
    /// The newest frame that must be decoded before this one.
    pub depends_on: u64,
    /// Microseconds since the session began, on the source's clock.
    pub capture_time: u64,
    /// Counts the chains of the session.
    pub chain: u32,
    /// Counts the sizes of the picture.
    pub epoch: u32,
    /// The picture in AVCC form, without parameter sets.
    pub sample: BatonBytes,
    /// The parameter sets: sequence sets first.
    pub sets: *const BatonBytes,
    /// How many parameter sets.
    pub set_count: usize,
    /// How many of the first `sets` are sequence parameter sets.
    pub sequence_sets: usize,
}

/// A message of the source about the pictures.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatonSourceMessage {
    /// The bytes of this struct.
    pub size: u32,
    /// `SOURCE_` kind.
    pub kind: u32,
    /// Config: the epoch.
    pub epoch: u32,
    /// Config: the codec bit.
    pub codec: u32,
    /// Config: the width of the picture.
    pub width: u32,
    /// Config: the height of the picture.
    pub height: u32,
    /// Config: the most frames a second.
    pub fps: u32,
    /// Pause: `PAUSE_`. Notice: `DROPPED_`.
    pub reason: u32,
    /// Notice: how many events were dropped.
    pub count: u32,
    /// Zero.
    pub reserved: u32,
}

/// How the decoder is doing.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BatonDecoderState {
    /// The bytes the platform may write.
    pub size: u32,
    /// The time a frame takes, smoothed, in microseconds.
    pub decode_us: u32,
    /// `THERMAL_`.
    pub thermal: u32,
    /// Zero.
    pub reserved: u32,
}

/// Fills in what the device can show.
pub type LimitsFn = unsafe extern "C" fn(context: *mut c_void, limits: *mut BatonLimits) -> i32;
/// Makes the decoder and the screen for a window.
pub type OpenFn = unsafe extern "C" fn(
    context: *mut c_void,
    window: *const BatonWindow,
    link: u64,
    screen: *mut BatonScreen,
    message: *mut BatonMessage,
) -> i32;
/// Decodes a frame.
pub type DecodeFn = unsafe extern "C" fn(screen: *mut c_void, frame: *const BatonFrame) -> i32;
/// Shows the picture of a frame.
pub type ShowFn = unsafe extern "C" fn(screen: *mut c_void, number: u64);
/// The source said something about the pictures.
pub type SourceFn = unsafe extern "C" fn(screen: *mut c_void, message: *const BatonSourceMessage);
/// Fills in how the decoder is doing.
pub type StateFn = unsafe extern "C" fn(screen: *mut c_void, state: *mut BatonDecoderState);
/// The session is over.
pub type EndedFn = unsafe extern "C" fn(screen: *mut c_void);
/// The screen is not needed any more.
pub type ReleaseFn = unsafe extern "C" fn(screen: *mut c_void);

/// What the platform registers. A function that is `None` here is a registration that is refused.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BatonViewerVTable {
    /// The version of the interface the platform was written for.
    pub version: u32,
    /// The bytes of this struct as the platform knows it.
    pub size: u32,
    /// The first argument of `limits` and `open`.
    pub context: *mut c_void,
    /// See [`LimitsFn`].
    pub limits: Option<LimitsFn>,
    /// See [`OpenFn`].
    pub open: Option<OpenFn>,
    /// See [`DecodeFn`].
    pub decode: Option<DecodeFn>,
    /// See [`ShowFn`].
    pub show: Option<ShowFn>,
    /// See [`SourceFn`].
    pub source: Option<SourceFn>,
    /// See [`StateFn`].
    pub state: Option<StateFn>,
    /// See [`EndedFn`].
    pub ended: Option<EndedFn>,
    /// See [`ReleaseFn`].
    pub release: Option<ReleaseFn>,
}
