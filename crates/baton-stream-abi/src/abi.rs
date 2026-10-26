// SPDX-License-Identifier: Apache-2.0

//! The structs, functions and numbers of `include/baton_stream_abi.h`, in the layout that the
//! header has. The header says what each of them means; the comments here only say where Rust
//! needs more than C does. `tests/header.rs` holds every size, offset and number below to the
//! header.

use std::ffi::c_void;

/// The version of the interface. It changes only when something is taken away or moved.
pub const ABI_VERSION: u32 = 2;

/// What a function of the interface returns when it worked, and what a call of the vtable returns
/// when it worked.
pub const OK: i32 = 0;
/// A null pointer, a wrong version or size, a function that is missing, or a wrong value.
pub const E_ARGUMENT: i32 = -1;
/// A viewer, or a sharer, was registered already.
pub const E_REGISTERED: i32 = -2;
/// A call of the vtable failed.
pub const E_PLATFORM: i32 = -3;
/// The link is not open, or nobody waits for the request.
pub const E_UNKNOWN_LINK: i32 = -4;
/// Called from inside a call of the vtable, on the thread that the call is made on.
pub const E_REENTRANT: i32 = -5;
/// The Rust side went wrong inside the call.
pub const E_PANIC: i32 = -6;

/// The codec bit of H.264 in Annex B form on the wire.
pub const CODEC_H264: u32 = 1;

/// What the person picked is one window.
pub const PICKED_WINDOW: u32 = 0;
/// What the person picked is a whole display, with every window on it.
pub const PICKED_DISPLAY: u32 = 1;

/// What `screen` returns when the system has not let the program capture the screen.
pub const SCREEN_NEEDS_PERMISSION: i32 = 1;
/// What `screen` returns when there is no display to give now, for example because none is on.
pub const SCREEN_UNAVAILABLE: i32 = 2;
/// What `screen` returns when the display was asked for with [`SCREEN_PRIVATE`] and the system has
/// not let the program hold the keyboard and the pointer.
pub const SCREEN_NEEDS_INPUT_PERMISSION: i32 = 3;

/// A flag of `screen`: the display is for a session that another device controls, so nobody at
/// this one may see or use it meanwhile.
pub const SCREEN_PRIVATE: u32 = 1;

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

/// Make no picture now; ask again.
pub const BEGIN_NOT_NOW: u32 = 0;
/// Make a key frame.
pub const BEGIN_KEY: u32 = 1;
/// Make a recovery frame.
pub const BEGIN_RECOVERY: u32 = 2;
/// Make a frame that follows the one before.
pub const BEGIN_DELTA: u32 = 3;

/// The picture went out.
pub const SENT: u32 = 0;
/// The picture is of no use any more: drop it and begin again.
pub const SENT_STALE: u32 = 1;
/// The picture is larger than the protocol allows: nothing went out.
pub const SENT_TOO_LARGE: u32 = 2;
/// The session is over: nothing went out.
pub const SENT_OVER: u32 = 3;

/// Another window has the keyboard, or the application is not in front.
pub const KEYBOARD_ELSEWHERE: u32 = 0;
/// The application is in front and the shared window is its focused window.
pub const KEYBOARD_SHARED: u32 = 1;
/// The platform cannot tell, which counts as not the shared window.
pub const KEYBOARD_UNKNOWN: u32 = 2;

/// A window in front of the shared one belongs to its application.
pub const FRONT_SAME_APPLICATION: u32 = 1;
/// A window in front of the shared one is on the layer of an open menu.
pub const FRONT_MENU: u32 = 2;

/// The most windows in front of the shared one that `surroundings` has room for.
pub const FRONTS_MAX: u32 = 128;

/// A post of a pointer.
pub const POST_POINTER: u32 = 1;
/// A post of a key.
pub const POST_KEY: u32 = 2;

/// The pointer moved, with a button down or not.
pub const POINTER_MOVE: u32 = 1;
/// A button went down.
pub const POINTER_DOWN: u32 = 2;
/// A button came up.
pub const POINTER_UP: u32 = 3;
/// A scroll.
pub const POINTER_SCROLL: u32 = 4;

/// The primary button, or a touch.
pub const BUTTON_LEFT: u32 = 0;
/// The secondary button.
pub const BUTTON_RIGHT: u32 = 1;
/// The middle button.
pub const BUTTON_MIDDLE: u32 = 2;

/// Shift is down.
pub const MOD_SHIFT: u32 = 1;
/// Control is down.
pub const MOD_CONTROL: u32 = 2;
/// Option is down.
pub const MOD_OPTION: u32 = 4;
/// Command is down.
pub const MOD_COMMAND: u32 = 8;
/// Caps lock is on.
pub const MOD_CAPS_LOCK: u32 = 16;

/// The most parameter sets that a picture is given with.
pub const SETS_MAX: u32 = 256;
/// The most bytes of a parameter set.
pub const SET_BYTES_MAX: u32 = 4096;

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

/// The window that the person said yes to, or the screen that this device asked another to start
/// sharing. The second is made before the other device offers it, so its size is not known:
/// `width` and `height` are nought, `control` is what the person wished for, and there is no title
/// and no application.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BatonWindow {
    /// The bytes of this struct.
    pub size: u32,
    /// The width of the window in pixels; nought for a screen that this device asked for.
    pub width: u32,
    /// The height of the window in pixels; nought for a screen that this device asked for.
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

/// What the viewer of the other device can take.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BatonTakes {
    /// The bytes the platform may read.
    pub size: u32,
    /// The widest picture in pixels.
    pub max_width: u32,
    /// The highest picture in pixels.
    pub max_height: u32,
    /// The most frames a second.
    pub max_fps: u32,
    /// The codecs its decoder takes, as bits.
    pub codecs: u32,
    /// Zero.
    pub reserved: u32,
}

/// The picture that the platform makes of a window.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BatonFormat {
    /// The bytes of this struct.
    pub size: u32,
    /// One `CODEC_` bit.
    pub codec: u32,
    /// The width of the picture in pixels.
    pub width: u32,
    /// The height of the picture in pixels.
    pub height: u32,
    /// The most frames a second.
    pub fps: u32,
    /// Zero.
    pub reserved: u32,
}

/// The window, or the display, that the person picked.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BatonPicked {
    /// The bytes of this struct.
    pub size: u32,
    /// The width of the window or the display in pixels.
    pub width: u32,
    /// The height of the window or the display in pixels.
    pub height: u32,
    /// `PICKED_WINDOW` or `PICKED_DISPLAY`. A platform written before this field had a name
    /// wrote zero here, which is a window.
    pub kind: u32,
    /// The platform's own pointer for the window or the display.
    pub context: *mut c_void,
    /// The title, or the name of the display. What it is called is not for a log.
    pub title: BatonBytes,
    /// The name of the application, empty for a display.
    pub app: BatonBytes,
}

/// A rectangle on the screen, in points.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BatonRect {
    /// The left edge, from the left edge of the main display.
    pub x: f64,
    /// The top edge, from the top edge of the main display.
    pub y: f64,
    /// The width.
    pub width: f64,
    /// The height.
    pub height: f64,
}

/// How the window was laid out in a picture.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BatonGeometry {
    /// The width of the window on the screen, in points.
    pub window_width: f64,
    /// The height of the window on the screen, in points.
    pub window_height: f64,
    /// The width of the picture in pixels.
    pub picture_width: u32,
    /// The height of the picture in pixels.
    pub picture_height: u32,
    /// Where the content of the window is in the picture, in pixels.
    pub content: BatonRect,
}

/// A picture that the encoder made.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BatonPicture {
    /// The bytes of this struct.
    pub size: u32,
    /// `FRAME_REFERENCED`, or nothing.
    pub flags: u32,
    /// When the window was captured, in microseconds on the clock of the session.
    pub capture_time: u64,
    /// The bytes of each length in `sample`: 1, 2 or 4.
    pub length_size: u32,
    /// Zero.
    pub reserved: u32,
    /// The picture in AVCC form.
    pub sample: BatonBytes,
    /// The parameter sets of the format of the encoder.
    pub sets: *const BatonBytes,
    /// How many parameter sets.
    pub set_count: usize,
    /// How the window was laid out in this picture. A struct that ends before it is read with a
    /// geometry of zero.
    pub geometry: BatonGeometry,
}

/// A window that is in front of the shared one.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BatonFront {
    /// Where it is.
    pub rect: BatonRect,
    /// `FRONT_` bits.
    pub flags: u32,
    /// Zero.
    pub reserved: u32,
}

/// What the Mac is doing around the window.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BatonSurroundings {
    /// The bytes the platform may write.
    pub size: u32,
    /// 1 if the Mac is locked or its display is off.
    pub locked: u32,
    /// 1 if the shared window is on screen.
    pub on_screen: u32,
    /// `KEYBOARD_`.
    pub keyboard: u32,
    /// Where the shared window is now.
    pub window: BatonRect,
    /// Room for `front_cap` windows, written by the platform.
    pub fronts: *mut BatonFront,
    /// How many windows there is room for.
    pub front_cap: u32,
    /// How many the platform wrote.
    pub front_count: u32,
}

/// One thing that the person did at the other device, put to the shared window.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BatonPost {
    /// The bytes of this struct.
    pub size: u32,
    /// `POST_`.
    pub kind: u32,
    /// A pointer: where, in points on the screen.
    pub x: f64,
    /// A pointer: where, in points on the screen.
    pub y: f64,
    /// A pointer: `POINTER_`.
    pub action: u32,
    /// A pointer that went down or came up: `BUTTON_`.
    pub button: u32,
    /// A scroll: along x, in points. Positive moves the content to the right.
    pub dx: f32,
    /// A scroll: along y, in points. Positive moves the content down.
    pub dy: f32,
    /// `MOD_` bits.
    pub modifiers: u32,
    /// A key: a USB HID usage of the keyboard page, or 0 for text with no key.
    pub code: u32,
    /// A key: 1 if it went down.
    pub down: u32,
    /// A key: 1 if the system repeats a key that is held.
    pub repeat: u32,
    /// A key: the text it produced. Not for a log.
    pub text: BatonBytes,
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

/// Whether the person can let another device control a window that they share.
pub type ControlFn = unsafe extern "C" fn(context: *mut c_void) -> u32;
/// Shows the picker of the system.
pub type PickFn =
    unsafe extern "C" fn(context: *mut c_void, request: u64, message: *mut BatonMessage) -> i32;
/// Takes the picker down.
pub type CancelPickFn = unsafe extern "C" fn(context: *mut c_void, request: u64);
/// Chooses the picture to begin with, within what the other device takes.
pub type PrepareFn = unsafe extern "C" fn(
    picked: *mut c_void,
    takes: *const BatonTakes,
    format: *mut BatonFormat,
) -> i32;
/// Starts to capture and encode the window.
pub type StartFn =
    unsafe extern "C" fn(picked: *mut c_void, link: u64, message: *mut BatonMessage) -> i32;
/// Fills in what the Mac is doing around the window.
pub type SurroundingsFn =
    unsafe extern "C" fn(picked: *mut c_void, surroundings: *mut BatonSurroundings) -> i32;
/// Puts one thing that the person did to the window.
pub type PostFn = unsafe extern "C" fn(picked: *mut c_void, post: *const BatonPost) -> i32;
/// Gives the main display, without the person choosing it.
pub type ScreenFn = unsafe extern "C" fn(
    context: *mut c_void,
    flags: u32,
    picked: *mut BatonPicked,
    message: *mut BatonMessage,
) -> i32;

/// What the platform registers to share windows. A function that is `None` here is a registration
/// that is refused, except `surroundings` and `post`, which a platform that cannot post input
/// leaves out, and `screen`, which a platform that cannot give its screen without a picker leaves
/// out. `ended` and `release` are called with the pointer of the window, where the viewer's are
/// called with the pointer of a screen.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BatonSharerVTable {
    /// The version of the interface the platform was written for.
    pub version: u32,
    /// The bytes of this struct as the platform knows it.
    pub size: u32,
    /// The first argument of `control`, `pick`, `cancel_pick` and `screen`.
    pub context: *mut c_void,
    /// See [`ControlFn`].
    pub control: Option<ControlFn>,
    /// See [`PickFn`].
    pub pick: Option<PickFn>,
    /// See [`CancelPickFn`].
    pub cancel_pick: Option<CancelPickFn>,
    /// See [`PrepareFn`].
    pub prepare: Option<PrepareFn>,
    /// See [`StartFn`].
    pub start: Option<StartFn>,
    /// See [`EndedFn`].
    pub ended: Option<EndedFn>,
    /// See [`ReleaseFn`].
    pub release: Option<ReleaseFn>,
    /// See [`SurroundingsFn`].
    pub surroundings: Option<SurroundingsFn>,
    /// See [`PostFn`].
    pub post: Option<PostFn>,
    /// See [`ScreenFn`]. A platform that cannot give its screen without a picker leaves it out.
    pub screen: Option<ScreenFn>,
}
