/* SPDX-License-Identifier: Apache-2.0 */

/*
 * The interface between the platform code of a device (Swift on an Apple device) and the Rust
 * side of Baton when a device shows a window that another device shares.
 *
 * The Rust side takes the pictures off the network and puts them in order. The platform decodes
 * them, draws them, and says how it is doing. This file is the whole of what they say to each
 * other, and docs/adr/0009-window-streaming.md says why it is this small. It is written by hand,
 * and the tests of the crate read it: the sizes and offsets that the BATON_STATIC_ASSERT lines at
 * the end hold are the ones that the Rust structs have, so a struct that changes on one side and
 * not on the other fails a test.
 *
 * Who calls what.
 *
 *   The platform calls three functions, once at launch and later:
 *     baton_stream_abi_version, baton_stream_register_viewer, baton_stream_link_decode_failed.
 *   The Rust side calls the functions of the vtable that the platform registered:
 *     limits and open (about the device), and decode, show, source, state, ended and release
 *     (about one screen, with the pointer that open gave).
 *
 * Rules that hold for every call.
 *
 *   - 64-bit targets only. The numbers below are those of a 64-bit target, and the Rust side
 *     does not build for another.
 *   - A pointer that the Rust side passes is valid until the call returns, and not after: copy
 *     what you keep. Text is UTF-8 and has no terminator; its length is in the BatonBytes.
 *   - A pointer that you pass must be valid for the call. The Rust side checks for null and for
 *     sizes and never reads more than a struct says it has.
 *   - A struct begins with its size in bytes, or with its version and its size. It tells the
 *     other side how much of it is there. Fields are only ever added at the end, so a side that
 *     knows fewer fields ignores the rest, and a side that knows more reads a missing field as
 *     zero. A field called reserved is zero when the Rust side writes it, and is ignored when it
 *     reads it.
 *   - No call waits for long. The Rust side makes the calls of a screen while it holds the lock
 *     of the session, so a decoder that takes time hands the picture to a thread of its own, and
 *     reports a failure that comes later with baton_stream_link_decode_failed.
 *   - A function of this interface that is called from inside a call of the vtable, on the same
 *     thread, is refused with BATON_E_REENTRANT. From another thread it is allowed, but the call
 *     of the vtable must not wait for that thread.
 *   - Threads. limits is called on the thread that registers, before the registration returns.
 *     open is called on a thread of the app's runtime. The calls of one screen never overlap,
 *     but two calls may come on two threads, so a screen keeps nothing in thread-local storage.
 *     The calls of two screens may overlap.
 *   - Nothing in the platform may unwind into the Rust side: a Swift error or an exception
 *     is caught before it leaves the call.
 */

#ifndef BATON_STREAM_ABI_H
#define BATON_STREAM_ABI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The version of this interface. It changes only when something is taken away or moved. */
#define BATON_STREAM_ABI_VERSION 1u

/* What a function of this interface returns, and what a call of the vtable returns. */
#define BATON_OK 0
/* A null pointer, a wrong version or size, a function that is missing, or a value that is wrong. */
#define BATON_E_ARGUMENT (-1)
/* A viewer was registered already. There is one for the life of the process. */
#define BATON_E_REGISTERED (-2)
/* A call of the vtable failed: it returned something else than BATON_OK. */
#define BATON_E_PLATFORM (-3)
/* The link is not one that is open: it was never given, or its screen was released. */
#define BATON_E_UNKNOWN_LINK (-4)
/* Called from inside a call of the vtable, on the thread that the call is made on. */
#define BATON_E_REENTRANT (-5)
/* The Rust side went wrong inside the call. Nothing it held is lost, but report it. */
#define BATON_E_PANIC (-6)

/* The codecs that a device can decode, as bits. */
#define BATON_CODEC_H264 1u

/* The flags of a frame. */
#define BATON_FRAME_KEY 1u
#define BATON_FRAME_RECOVERY 2u
#define BATON_FRAME_REFERENCED 4u

/* The kinds of message that the source sends. */
#define BATON_SOURCE_CONFIG 1u
#define BATON_SOURCE_PAUSE 2u
#define BATON_SOURCE_RESUME 3u
#define BATON_SOURCE_NOTICE 4u

/* Why the source paused. Any reason that this version does not know reads as OTHER. */
#define BATON_PAUSE_OTHER 0u
#define BATON_PAUSE_LOCKED 1u
#define BATON_PAUSE_HIDDEN 2u

/*
 * Why the source dropped events that the viewer sent, in a notice. Any reason that this version
 * does not know reads as OTHER.
 */
#define BATON_DROPPED_OTHER 0u
#define BATON_DROPPED_MALFORMED 1u
#define BATON_DROPPED_CONTROL_OFF 2u
#define BATON_DROPPED_PAUSED 3u
#define BATON_DROPPED_LOCKED 4u
#define BATON_DROPPED_NOT_ON_SCREEN 5u
#define BATON_DROPPED_TOO_FAST 6u
#define BATON_DROPPED_FRAME_UNKNOWN 7u
#define BATON_DROPPED_PICTURE_OUT_OF_DATE 8u
#define BATON_DROPPED_WINDOW_RESIZED 9u
#define BATON_DROPPED_OUTSIDE_WINDOW 10u
#define BATON_DROPPED_COVERED 11u
#define BATON_DROPPED_NO_KEYBOARD 12u
#define BATON_DROPPED_APP_WINDOW_IN_FRONT 13u
#define BATON_DROPPED_TOO_MANY_HELD 14u
#define BATON_DROPPED_NOT_PRESSED 15u

/* How hot the device is, as the system reports it. Any other value reads as OTHER. */
#define BATON_THERMAL_NOMINAL 0u
#define BATON_THERMAL_FAIR 1u
#define BATON_THERMAL_SERIOUS 2u
#define BATON_THERMAL_CRITICAL 3u
#define BATON_THERMAL_OTHER 4u

/* The most bytes of a message that explains why a window could not be shown. */
#define BATON_MESSAGE_MAX 256u

/* Bytes in memory that the other side owns: text (UTF-8, no terminator) or a picture. */
typedef struct BatonBytes {
    const uint8_t *ptr;
    size_t len;
} BatonBytes;

/*
 * What the device can show. The platform fills it in limits. The app checks the numbers against
 * what the protocol allows (the largest picture is 3840 by 3840, the most frames a second 120,
 * and there are one to eight codecs) and does not start if they are wrong. A codec that has no
 * bit above is an error.
 */
typedef struct BatonLimits {
    uint32_t size;       /* written by the Rust side: the bytes the platform may write */
    uint32_t max_width;  /* the widest picture in pixels */
    uint32_t max_height; /* the highest picture in pixels */
    uint32_t max_fps;    /* the most frames a second, at most 65535 */
    uint32_t codecs;     /* the codecs the decoder takes, as BATON_CODEC_ bits */
    uint32_t reserved;
} BatonLimits;

/* The window that another device offered and the person said yes to. */
typedef struct BatonWindow {
    uint32_t size;
    uint32_t width;       /* the size of the window in pixels */
    uint32_t height;
    uint32_t control;     /* 1 if the person at the other device lets this device control it */
    BatonBytes title;     /* may be empty. It names a document, a person or a page: do not log it */
    BatonBytes app;       /* the application the window belongs to; may be empty */
} BatonWindow;

/*
 * The screen that open makes: the platform's own pointer for it, which the Rust side hands back as
 * the first argument of every call about this screen, and the number that the user interface
 * draws it with (on Flutter, the id of the texture). The platform keeps the pointer valid until
 * release.
 */
typedef struct BatonScreen {
    uint32_t size;
    uint32_t reserved;
    void *context;
    int64_t id;
} BatonScreen;

/*
 * Room for the sentence that tells the person why the window could not be shown, when open
 * fails. The platform writes at most cap bytes of UTF-8 to text and sets len. It is shown to
 * the person, so it says what happened in plain words, and it is cut to one line.
 */
typedef struct BatonMessage {
    uint32_t size;
    uint32_t cap;
    uint32_t len;
    uint32_t reserved;
    uint8_t *text;
} BatonMessage;

/*
 * A frame to decode, in the order in which frames are to be decoded.
 *
 * sample is the picture in the form that the Apple decoder takes (AVCC): the NAL units behind
 * lengths of four bytes in network order, without the parameter sets, the delimiters and the
 * filler. sets holds the parameter sets that the frame came with, sequence parameter sets first
 * (sequence_sets of them) and then picture parameter sets, each without a start code, and is NULL
 * when there are none. A key frame always has at least one of each, and the Rust side checks that
 * the picture they describe is not larger than limits said the device takes. Another frame
 * usually has none; one that does has the same checks. The platform makes the description of the
 * format from them whenever they differ from the last ones.
 */
typedef struct BatonFrame {
    uint32_t size;
    uint32_t flags;          /* BATON_FRAME_ bits */
    uint64_t number;         /* counts the frames of the session from 0 */
    uint64_t depends_on;     /* the newest frame that must be decoded before this one */
    uint64_t capture_time;   /* microseconds since the session began, on the source's clock */
    uint32_t chain;
    uint32_t epoch;          /* counts the sizes of the picture */
    BatonBytes sample;
    const BatonBytes *sets;  /* set_count entries */
    size_t set_count;
    size_t sequence_sets;    /* how many of the first sets are sequence parameter sets */
} BatonFrame;

/*
 * A message of the source about the pictures. Fields that the kind does not use are zero.
 *
 *   BATON_SOURCE_CONFIG: the pictures that follow have this epoch, codec, size and top rate.
 *                        Their key frames can arrive before this message does.
 *   BATON_SOURCE_PAUSE:  no pictures for a while, for the reason.
 *   BATON_SOURCE_RESUME: pictures go on.
 *   BATON_SOURCE_NOTICE: the source dropped count events of this device, for the reason.
 */
typedef struct BatonSourceMessage {
    uint32_t size;
    uint32_t kind;     /* BATON_SOURCE_ */
    uint32_t epoch;
    uint32_t codec;    /* BATON_CODEC_ */
    uint32_t width;
    uint32_t height;
    uint32_t fps;
    uint32_t reason;   /* BATON_PAUSE_ for a pause, BATON_DROPPED_ for a notice */
    uint32_t count;
    uint32_t reserved;
} BatonSourceMessage;

/* How the decoder is doing. The platform fills it in state; the Rust side sends it as a report. */
typedef struct BatonDecoderState {
    uint32_t size;
    uint32_t decode_us; /* the time a frame takes, smoothed, in microseconds */
    uint32_t thermal;   /* BATON_THERMAL_ */
    uint32_t reserved;
} BatonDecoderState;

/*
 * The functions of the platform. All of them are mandatory. A function that returns int32_t
 * returns BATON_OK, or something else for a failure.
 *
 *   limits:  fills in what the device can show. Called once, at registration.
 *   open:    the person said yes to a window: makes the decoder and the screen for it, and fills
 *            in screen, whose context is not NULL. link is the number to give
 *            baton_stream_link_decode_failed for this screen, and it is valid until release. On a
 *            failure fills in message if it can; a success without a context is a failure too.
 *            Does not wait: a platform that needs time fails here, with a sentence.
 *   decode:  decodes the frame. A failure means that this frame cannot be decoded, and the Rust
 *            side asks the source for another key frame.
 *   show:    shows the picture of frame number, the newest that decode took.
 *   source:  the source said something about the pictures.
 *   state:   fills in how the decoder is doing.
 *   ended:   the session is over. Let go of what the screen shows; no call follows except release.
 *   release: the screen is not needed any more, whether or not ended came first. Let go of all of
 *            it. Called exactly once, and nothing follows. The link is not valid after it.
 */
typedef int32_t (*BatonLimitsFn)(void *context, BatonLimits *limits);
typedef int32_t (*BatonOpenFn)(void *context, const BatonWindow *window, uint64_t link,
                               BatonScreen *screen, BatonMessage *message);
typedef int32_t (*BatonDecodeFn)(void *screen, const BatonFrame *frame);
typedef void (*BatonShowFn)(void *screen, uint64_t number);
typedef void (*BatonSourceFn)(void *screen, const BatonSourceMessage *message);
typedef void (*BatonStateFn)(void *screen, BatonDecoderState *state);
typedef void (*BatonEndedFn)(void *screen);
typedef void (*BatonReleaseFn)(void *screen);

/*
 * What the platform registers: the version of the interface it was written for, the size of this
 * struct as the platform knows it, the pointer that limits and open get as their first argument,
 * and the functions. A struct that is larger than the Rust side knows is accepted, and the end of
 * it is ignored.
 */
typedef struct BatonViewerVTable {
    uint32_t version;
    uint32_t size;
    void *context;
    BatonLimitsFn limits;
    BatonOpenFn open;
    BatonDecodeFn decode;
    BatonShowFn show;
    BatonSourceFn source;
    BatonStateFn state;
    BatonEndedFn ended;
    BatonReleaseFn release;
} BatonViewerVTable;

/* The version of the interface that the library was built with, for the platform to compare. */
uint32_t baton_stream_abi_version(void);

/*
 * Registers the platform's viewer. Call it once, at launch, before the app starts: a device that
 * has registered one takes windows that other devices offer, and a device that has not takes
 * none. The vtable is copied; the platform keeps whatever its context points to for as long as the
 * process lives. Returns BATON_OK, or BATON_E_ARGUMENT (the vtable or the limits it gave are
 * wrong), BATON_E_REGISTERED, BATON_E_PLATFORM (limits failed), BATON_E_REENTRANT or
 * BATON_E_PANIC. A registration that fails leaves nothing registered.
 */
int32_t baton_stream_register_viewer(const BatonViewerVTable *vtable);

/*
 * The decoder of a screen failed on a frame after decode had returned BATON_OK for it, from a
 * thread of its own. Nothing can be decoded until a key frame comes, so the Rust side asks the
 * source for one. A decoder that fails inside decode returns the failure from decode instead.
 * Returns BATON_OK also when the session is not running yet or is over (nothing happens then), or
 * BATON_E_UNKNOWN_LINK, BATON_E_REENTRANT or BATON_E_PANIC.
 */
int32_t baton_stream_link_decode_failed(uint64_t link);

/*
 * The layout. These lines make a compiler that disagrees with the numbers refuse to build, and
 * the tests of the crate hold the numbers to the Rust structs.
 */
#ifdef __cplusplus
#define BATON_STATIC_ASSERT(condition, message) static_assert(condition, message)
#else
#define BATON_STATIC_ASSERT(condition, message) _Static_assert(condition, message)
#endif

BATON_STATIC_ASSERT(sizeof(void *) == 8, "the interface is for 64-bit targets");
BATON_STATIC_ASSERT(sizeof(size_t) == 8, "the interface is for 64-bit targets");

BATON_STATIC_ASSERT(sizeof(BatonBytes) == 16, "BatonBytes");
BATON_STATIC_ASSERT(offsetof(BatonBytes, ptr) == 0, "BatonBytes.ptr");
BATON_STATIC_ASSERT(offsetof(BatonBytes, len) == 8, "BatonBytes.len");

BATON_STATIC_ASSERT(sizeof(BatonLimits) == 24, "BatonLimits");
BATON_STATIC_ASSERT(offsetof(BatonLimits, size) == 0, "BatonLimits.size");
BATON_STATIC_ASSERT(offsetof(BatonLimits, max_width) == 4, "BatonLimits.max_width");
BATON_STATIC_ASSERT(offsetof(BatonLimits, max_height) == 8, "BatonLimits.max_height");
BATON_STATIC_ASSERT(offsetof(BatonLimits, max_fps) == 12, "BatonLimits.max_fps");
BATON_STATIC_ASSERT(offsetof(BatonLimits, codecs) == 16, "BatonLimits.codecs");
BATON_STATIC_ASSERT(offsetof(BatonLimits, reserved) == 20, "BatonLimits.reserved");

BATON_STATIC_ASSERT(sizeof(BatonWindow) == 48, "BatonWindow");
BATON_STATIC_ASSERT(offsetof(BatonWindow, size) == 0, "BatonWindow.size");
BATON_STATIC_ASSERT(offsetof(BatonWindow, width) == 4, "BatonWindow.width");
BATON_STATIC_ASSERT(offsetof(BatonWindow, height) == 8, "BatonWindow.height");
BATON_STATIC_ASSERT(offsetof(BatonWindow, control) == 12, "BatonWindow.control");
BATON_STATIC_ASSERT(offsetof(BatonWindow, title) == 16, "BatonWindow.title");
BATON_STATIC_ASSERT(offsetof(BatonWindow, app) == 32, "BatonWindow.app");

BATON_STATIC_ASSERT(sizeof(BatonScreen) == 24, "BatonScreen");
BATON_STATIC_ASSERT(offsetof(BatonScreen, size) == 0, "BatonScreen.size");
BATON_STATIC_ASSERT(offsetof(BatonScreen, reserved) == 4, "BatonScreen.reserved");
BATON_STATIC_ASSERT(offsetof(BatonScreen, context) == 8, "BatonScreen.context");
BATON_STATIC_ASSERT(offsetof(BatonScreen, id) == 16, "BatonScreen.id");

BATON_STATIC_ASSERT(sizeof(BatonMessage) == 24, "BatonMessage");
BATON_STATIC_ASSERT(offsetof(BatonMessage, size) == 0, "BatonMessage.size");
BATON_STATIC_ASSERT(offsetof(BatonMessage, cap) == 4, "BatonMessage.cap");
BATON_STATIC_ASSERT(offsetof(BatonMessage, len) == 8, "BatonMessage.len");
BATON_STATIC_ASSERT(offsetof(BatonMessage, reserved) == 12, "BatonMessage.reserved");
BATON_STATIC_ASSERT(offsetof(BatonMessage, text) == 16, "BatonMessage.text");

BATON_STATIC_ASSERT(sizeof(BatonFrame) == 80, "BatonFrame");
BATON_STATIC_ASSERT(offsetof(BatonFrame, size) == 0, "BatonFrame.size");
BATON_STATIC_ASSERT(offsetof(BatonFrame, flags) == 4, "BatonFrame.flags");
BATON_STATIC_ASSERT(offsetof(BatonFrame, number) == 8, "BatonFrame.number");
BATON_STATIC_ASSERT(offsetof(BatonFrame, depends_on) == 16, "BatonFrame.depends_on");
BATON_STATIC_ASSERT(offsetof(BatonFrame, capture_time) == 24, "BatonFrame.capture_time");
BATON_STATIC_ASSERT(offsetof(BatonFrame, chain) == 32, "BatonFrame.chain");
BATON_STATIC_ASSERT(offsetof(BatonFrame, epoch) == 36, "BatonFrame.epoch");
BATON_STATIC_ASSERT(offsetof(BatonFrame, sample) == 40, "BatonFrame.sample");
BATON_STATIC_ASSERT(offsetof(BatonFrame, sets) == 56, "BatonFrame.sets");
BATON_STATIC_ASSERT(offsetof(BatonFrame, set_count) == 64, "BatonFrame.set_count");
BATON_STATIC_ASSERT(offsetof(BatonFrame, sequence_sets) == 72, "BatonFrame.sequence_sets");

BATON_STATIC_ASSERT(sizeof(BatonSourceMessage) == 40, "BatonSourceMessage");
BATON_STATIC_ASSERT(offsetof(BatonSourceMessage, size) == 0, "BatonSourceMessage.size");
BATON_STATIC_ASSERT(offsetof(BatonSourceMessage, kind) == 4, "BatonSourceMessage.kind");
BATON_STATIC_ASSERT(offsetof(BatonSourceMessage, epoch) == 8, "BatonSourceMessage.epoch");
BATON_STATIC_ASSERT(offsetof(BatonSourceMessage, codec) == 12, "BatonSourceMessage.codec");
BATON_STATIC_ASSERT(offsetof(BatonSourceMessage, width) == 16, "BatonSourceMessage.width");
BATON_STATIC_ASSERT(offsetof(BatonSourceMessage, height) == 20, "BatonSourceMessage.height");
BATON_STATIC_ASSERT(offsetof(BatonSourceMessage, fps) == 24, "BatonSourceMessage.fps");
BATON_STATIC_ASSERT(offsetof(BatonSourceMessage, reason) == 28, "BatonSourceMessage.reason");
BATON_STATIC_ASSERT(offsetof(BatonSourceMessage, count) == 32, "BatonSourceMessage.count");
BATON_STATIC_ASSERT(offsetof(BatonSourceMessage, reserved) == 36, "BatonSourceMessage.reserved");

BATON_STATIC_ASSERT(sizeof(BatonDecoderState) == 16, "BatonDecoderState");
BATON_STATIC_ASSERT(offsetof(BatonDecoderState, size) == 0, "BatonDecoderState.size");
BATON_STATIC_ASSERT(offsetof(BatonDecoderState, decode_us) == 4, "BatonDecoderState.decode_us");
BATON_STATIC_ASSERT(offsetof(BatonDecoderState, thermal) == 8, "BatonDecoderState.thermal");
BATON_STATIC_ASSERT(offsetof(BatonDecoderState, reserved) == 12, "BatonDecoderState.reserved");

BATON_STATIC_ASSERT(sizeof(BatonViewerVTable) == 80, "BatonViewerVTable");
BATON_STATIC_ASSERT(offsetof(BatonViewerVTable, version) == 0, "BatonViewerVTable.version");
BATON_STATIC_ASSERT(offsetof(BatonViewerVTable, size) == 4, "BatonViewerVTable.size");
BATON_STATIC_ASSERT(offsetof(BatonViewerVTable, context) == 8, "BatonViewerVTable.context");
BATON_STATIC_ASSERT(offsetof(BatonViewerVTable, limits) == 16, "BatonViewerVTable.limits");
BATON_STATIC_ASSERT(offsetof(BatonViewerVTable, open) == 24, "BatonViewerVTable.open");
BATON_STATIC_ASSERT(offsetof(BatonViewerVTable, decode) == 32, "BatonViewerVTable.decode");
BATON_STATIC_ASSERT(offsetof(BatonViewerVTable, show) == 40, "BatonViewerVTable.show");
BATON_STATIC_ASSERT(offsetof(BatonViewerVTable, source) == 48, "BatonViewerVTable.source");
BATON_STATIC_ASSERT(offsetof(BatonViewerVTable, state) == 56, "BatonViewerVTable.state");
BATON_STATIC_ASSERT(offsetof(BatonViewerVTable, ended) == 64, "BatonViewerVTable.ended");
BATON_STATIC_ASSERT(offsetof(BatonViewerVTable, release) == 72, "BatonViewerVTable.release");

#ifdef __cplusplus
}
#endif

#endif /* BATON_STREAM_ABI_H */
