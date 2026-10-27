/* SPDX-License-Identifier: Apache-2.0 */

/*
 * The interface between the platform code of a device (Swift on an Apple device) and the Rust
 * side of Baton, for the two things that a device does with windows: it shows a window that
 * another device shares, and it shares a window of its own.
 *
 * Showing. The Rust side takes the pictures off the network and puts them in order. The platform
 * decodes them, draws them, and says how it is doing.
 *
 * Sharing. The person picks a window, or a whole display, with the picker of the system, and the
 * platform captures it and encodes it. The Rust side offers it to the other device, decides when
 * a picture may be made and how it goes out, and sends it.
 *
 * Sound. A whole display can go with the sound of the device. The Rust side tells the sharing
 * platform when the other device wants to hear it (hear); the platform records it, encodes it,
 * and gives each packet to the Rust side, which sends it. The showing platform is given the
 * packets in order (sound, play, stopped), decodes them and plays them. Both halves are optional:
 * a platform that leaves them out sends no sound, or plays none.
 *
 * This file is the whole of what the two sides say to each other, and
 * docs/adr/0009-window-streaming.md says why it is this small. It is written by hand, and the
 * tests of the crate read it: the sizes and offsets that the BATON_STATIC_ASSERT lines at the
 * end hold are the ones that the Rust structs have, so a struct that changes on one side and not
 * on the other fails a test.
 *
 * Who calls what.
 *
 *   The platform calls these functions of the Rust side:
 *     at launch: baton_stream_abi_version, baton_stream_register_viewer and
 *       baton_stream_register_sharer;
 *     while a window is shown: baton_stream_link_decode_failed, and, for a platform that plays
 *       sound, baton_stream_link_sound_failed;
 *     while the person picks a window or a display to share: baton_stream_pick_done;
 *     while a window is shared: the baton_stream_source_ functions, those of the sound among
 *       them (baton_stream_source_sound_begin, baton_stream_source_sound_silent and
 *       baton_stream_source_sound).
 *   The Rust side calls the functions of the vtables that the platform registered:
 *     viewer: limits and open (about the device), and decode, show, source, state, ended and
 *       release (about one screen, with the pointer that open gave), and, for a platform that
 *       plays sound, sound, play and stopped (about the same screen);
 *     sharer: control, pick, cancel_pick and screen (about the device), and prepare, start,
 *       surroundings, post, ended and release (about one window, with the pointer that
 *       baton_stream_pick_done or screen gave), and, for a platform that keeps a screen
 *       private, lift, for one that sends sound, hear, and for one that can end a private
 *       session without locking the device, leave (about the same window).
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
 *     reads it. BatonBytes, a pointer and a length, is the one struct that does not begin so.
 *   - No call waits for long. The Rust side makes the calls of a screen while it holds the lock
 *     of the session, so a decoder that takes time hands the picture to a thread of its own, and
 *     reports a failure that comes later with baton_stream_link_decode_failed, and a speaker
 *     that decodes sound hands the packets to a thread of its own too, and reports a run that it
 *     could not make with baton_stream_link_sound_failed. The calls of a window that is shared
 *     hold no lock that a function of this interface needs, so ended and release may stop the
 *     platform's capture threads, also ones that are inside a function of this interface at that
 *     moment.
 *   - A function of this interface that is called from inside a call of a vtable, on the same
 *     thread, is refused with BATON_E_REENTRANT. From another thread it is allowed, but the call
 *     of the vtable must not wait for that thread.
 *   - An out argument of a function of this interface, a pointer that the Rust side writes
 *     through, is written only when the function returns BATON_OK. It is never read, so it need
 *     not be set.
 *   - Threads. limits is called on the thread that registers, before the registration returns.
 *     open is called on a thread of the app's runtime, and two calls of open may overlap each
 *     other and the calls of any screen that is open, so whatever open shares with other calls
 *     (a table of screens, a pool of textures) is kept under a lock. The calls of one screen
 *     (decode, show, source, state, sound, play, stopped, ended, release) never overlap, but two
 *     calls may come on two threads, so a screen keeps nothing in thread-local storage. The
 *     calls of two screens may overlap. The same holds for the sharer: control, pick and
 *     cancel_pick are called on a thread of the app's runtime and may overlap each other; the
 *     calls of one window (prepare, start, surroundings, post, lift, hear, ended or leave,
 *     release) never overlap, and release is the last; the calls of two windows may overlap.
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
#define BATON_STREAM_ABI_VERSION 2u

/* What a function of this interface returns, and what a call of the vtable returns. */
#define BATON_OK 0
/* A null pointer, a wrong version or size, a function that is missing, or a value that is wrong. */
#define BATON_E_ARGUMENT (-1)
/* A viewer, or a sharer, was registered already; there is one of each for the process. */
#define BATON_E_REGISTERED (-2)
/* A call of the vtable failed: it returned something else than BATON_OK. */
#define BATON_E_PLATFORM (-3)
/*
 * The link is not one that is open: it was never given, or its screen was released, or its
 * session is over. The same for the request of a pick that nobody waits for.
 */
#define BATON_E_UNKNOWN_LINK (-4)
/* Called from inside a call of the vtable, on the thread that the call is made on. */
#define BATON_E_REENTRANT (-5)
/* The Rust side went wrong inside the call. Nothing it held is lost, but report it. */
#define BATON_E_PANIC (-6)

/* The codecs that a device can decode, as bits. */
#define BATON_CODEC_H264 1u

/* What the person picked, as BatonPicked.kind says. */
#define BATON_PICKED_WINDOW 0u
#define BATON_PICKED_DISPLAY 1u

/*
 * What screen returns when it did not give the display, besides BATON_OK: the system has not let
 * the program capture the screen, there is no display to give now, or the display was asked for
 * with BATON_SCREEN_PRIVATE and the system has not let the program hold the keyboard and the
 * pointer. The last one means nothing for a display that was not asked for so, and counts as a
 * failure then. Any other value is a failure, with the sentence in message if the platform wrote
 * one.
 */
#define BATON_SCREEN_NEEDS_PERMISSION 1
#define BATON_SCREEN_UNAVAILABLE 2
#define BATON_SCREEN_NEEDS_INPUT_PERMISSION 3

/* The flags of screen. */
#define BATON_SCREEN_PRIVATE 1u

/* The bits of lift. */
#define BATON_LIFT_SCREEN 1u
#define BATON_LIFT_INPUT 2u

/* The flags of a frame. */
#define BATON_FRAME_KEY 1u
#define BATON_FRAME_RECOVERY 2u
#define BATON_FRAME_REFERENCED 4u

/* The kinds of message that the source sends. */
#define BATON_SOURCE_CONFIG 1u
#define BATON_SOURCE_PAUSE 2u
#define BATON_SOURCE_RESUME 3u
#define BATON_SOURCE_NOTICE 4u

/*
 * Why the source paused. On the viewing side, a reason that this version does not know reads as
 * OTHER; baton_stream_source_pause refuses one with BATON_E_ARGUMENT.
 */
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

/* The most bytes of a message that explains why a window could not be shown or shared. */
#define BATON_MESSAGE_MAX 256u

/* What baton_stream_source_begin says about the next picture. */
#define BATON_BEGIN_NOT_NOW 0u  /* make no picture now; ask again */
#define BATON_BEGIN_KEY 1u      /* make a key frame */
#define BATON_BEGIN_RECOVERY 2u /* make a recovery frame; a platform that holds no long-term
                                   references is never asked for one, and makes a key frame */
#define BATON_BEGIN_DELTA 3u    /* make a frame that follows the one before */

/* The most parameter sets that a picture is given with, and the most bytes of one. */
#define BATON_SETS_MAX 256u
#define BATON_SET_BYTES_MAX 4096u

/* What baton_stream_source_sent says became of the picture. */
#define BATON_SENT 0u           /* it went out; the number says which frame it is */
#define BATON_SENT_STALE 1u     /* it is of no use any more: drop it and begin again */
#define BATON_SENT_TOO_LARGE 2u /* it is larger than the protocol allows: nothing went out */
#define BATON_SENT_OVER 3u      /* the session is over: nothing went out */

/* The codecs of sound, in BatonSound. Any other number is one that this version does not know. */
#define BATON_SOUND_CODEC_AAC 1u /* AAC-LC: a packet is one access unit, with no ADTS header */

/* The flags of a BatonPlay. */
#define BATON_PLAY_FIRST 1u /* it begins a run of sound: the decoder starts afresh at it */

/*
 * Why there is no sound, as stopped says it, and as baton_stream_source_sound_silent is told it.
 * Only the last three are the platform's to say; the first two are given to stopped and no other
 * call. A reason that this version does not know reads as OTHER.
 */
#define BATON_SILENT_OFF 0u           /* the person turned the sound off */
#define BATON_SILENT_WINDOW 1u        /* the screen is a window, and only a display has sound */
#define BATON_SILENT_NO_PERMISSION 2u /* the system has not let the program record the sound */
#define BATON_SILENT_FAILED 3u        /* something went wrong where the sound is recorded */
#define BATON_SILENT_OTHER 4u         /* any other reason, also that the person there does not
                                          let this device hear */

/* What baton_stream_source_sound says became of the packet. */
#define BATON_SOUND_SENT 0u      /* it went out; the number says which packet it is */
#define BATON_SOUND_DROPPED 1u   /* it did not go out, and nothing is wrong: drop it */
#define BATON_SOUND_TOO_LARGE 2u /* it is larger than the protocol allows: nothing went out */
#define BATON_SOUND_OVER 3u      /* the session is over: nothing went out */

/* Whether the shared window has the keyboard, in BatonSurroundings. */
#define BATON_KEYBOARD_ELSEWHERE 0u /* another window has it, or the application is not in front */
#define BATON_KEYBOARD_SHARED 1u    /* the application is in front and the window is its focus */
#define BATON_KEYBOARD_UNKNOWN 2u   /* the platform cannot tell, which counts as not the window */

/* The flags of a window that is in front of the shared one, in BatonFront. */
#define BATON_FRONT_SAME_APPLICATION 1u /* it belongs to the application of the shared window */
#define BATON_FRONT_MENU 2u             /* it is on the layer of an open menu */

/* The most windows in front of the shared one that surroundings has room for. */
#define BATON_FRONTS_MAX 128u

/* What a BatonPost puts to the window. */
#define BATON_POST_POINTER 1u
#define BATON_POST_KEY 2u

/* What a pointer did, in a post of a pointer. */
#define BATON_POINTER_MOVE 1u   /* it moved, with a button down or not */
#define BATON_POINTER_DOWN 2u   /* a button went down */
#define BATON_POINTER_UP 3u     /* a button came up */
#define BATON_POINTER_SCROLL 4u /* a scroll, by dx and dy */

/* The buttons of a pointer. */
#define BATON_BUTTON_LEFT 0u /* the primary button, or a touch */
#define BATON_BUTTON_RIGHT 1u
#define BATON_BUTTON_MIDDLE 2u

/* The modifier keys that are down, as bits. */
#define BATON_MOD_SHIFT 1u
#define BATON_MOD_CONTROL 2u
#define BATON_MOD_OPTION 4u
#define BATON_MOD_COMMAND 8u
#define BATON_MOD_CAPS_LOCK 16u

/*
 * Bytes in memory that the other side owns: text (UTF-8, no terminator) or a picture. When len is
 * 0, ptr is not to be read and may be any value, NULL or not.
 */
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

/*
 * The window that another device offered and the person said yes to, or the screen that this
 * device asked another one to start sharing. The second is made before the other device offers
 * it, so its size is not known: width and height are 0, control is what the person wished for
 * (the other device may not grant it), and title and app are empty. The size of the picture comes
 * with the first picture.
 */
typedef struct BatonWindow {
    uint32_t size;
    uint32_t width;       /* the size of the window in pixels; 0 for a screen asked for */
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
 * What the viewer of the other device can take: the largest picture, the highest rate and the
 * codecs, as it said when it accepted the window. The picture that prepare chooses stays within
 * it.
 */
typedef struct BatonTakes {
    uint32_t size;       /* written by the Rust side: the bytes the platform may read */
    uint32_t max_width;  /* the widest picture in pixels */
    uint32_t max_height; /* the highest picture in pixels */
    uint32_t max_fps;    /* the most frames a second */
    uint32_t codecs;     /* the codecs its decoder takes, as BATON_CODEC_ bits */
    uint32_t reserved;
} BatonTakes;

/*
 * The picture that the platform makes of a window: how it is coded and how large it is. The
 * picture shows the whole window, scaled to this size. prepare chooses the first one, and
 * baton_stream_source_new_epoch gives each later one, which begins with a key frame. The numbers
 * are held to what the protocol allows (an edge from 1 to 3840 pixels, a rate from 1 to 120) and
 * to what the viewer takes, and a format beyond either is refused.
 */
typedef struct BatonFormat {
    uint32_t size;
    uint32_t codec;  /* one BATON_CODEC_ bit */
    uint32_t width;  /* the size of the picture in pixels */
    uint32_t height;
    uint32_t fps;    /* the most frames a second that the platform makes */
    uint32_t reserved;
} BatonFormat;

/*
 * The window, or the whole display, that the person picked, as baton_stream_pick_done gives it.
 * context is the platform's own pointer for it, which the Rust side hands back as the first
 * argument of every call about it until release, and is not NULL. The platform keeps it valid
 * until then.
 *
 * kind is BATON_PICKED_WINDOW or BATON_PICKED_DISPLAY; a platform that was written before the
 * field had a name wrote zero there, which is a window, and any other value is refused with
 * BATON_E_ARGUMENT. A display is everything on one screen, every window on it included.
 *
 * width and height are the size of the window, or of the display, in pixels, from 1; one larger
 * than 16384 reads as 16384. It may be larger than any picture that is sent, which is scaled
 * down. title and app are copied before the call returns. They are cut to 512 and 256 bytes, and
 * may be empty. A title names a document, a person or a page: do not log it. A display has no
 * application, so app is empty, and its title is its name, if the platform knows one.
 */
typedef struct BatonPicked {
    uint32_t size;
    uint32_t width;
    uint32_t height;
    uint32_t kind;
    void *context;
    BatonBytes title;
    BatonBytes app;
} BatonPicked;

/*
 * A rectangle on the screen, in points: the corner at the top left, measured from the top left
 * corner of the main display, with y growing downward. Where a window is and where something
 * is put to it are all given in this one space.
 */
typedef struct BatonRect {
    double x;
    double y;
    double width;
    double height;
} BatonRect;

/*
 * How the window was laid out in a picture, which is what turns a place in the picture into a
 * place on the screen when the person at the other device touches it. The platform writes it for
 * each picture that it hands over, from the window as it was when the picture was captured.
 * window_width and window_height are the size of the window on the screen, in points;
 * picture_width and picture_height are the size of the picture in pixels, as the encoder made it;
 * content is where the content of the window is in the picture, in pixels, and the rest is margin
 * that the platform added to keep the shape of the window. A geometry with a size that is not a
 * number above zero, as one that is all zero, says nothing: the picture is sent all the same, and
 * what the person touches on it is dropped. A platform that cannot post input leaves it zero.
 */
typedef struct BatonGeometry {
    double window_width;
    double window_height;
    uint32_t picture_width;
    uint32_t picture_height;
    BatonRect content;
} BatonGeometry;

/*
 * A picture that the encoder made, as baton_stream_source_sent gives it.
 *
 * sample is the picture in the form that the Apple encoder makes (AVCC): NAL units behind lengths
 * of length_size bytes, 1, 2 or 4, in network order. Parameter sets, delimiters and filler that it
 * holds are not sent. sets holds the parameter sets of the format of the encoder, sequence and
 * picture parameter sets in any order, each without a start code, and is NULL when there are none.
 * A key frame (one that holds a slice of an IDR picture) always goes out with them, so the
 * platform gives them with it and has them checked: at least one of each. Any other picture is
 * sent without them, so the platform may give them with every picture and they are not read. At
 * most BATON_SETS_MAX of them, of at most BATON_SET_BYTES_MAX bytes each.
 *
 * flags holds BATON_FRAME_REFERENCED if a later frame may refer to this one. The kind of the frame
 * is the one that baton_stream_source_begin said, so the other bits are ignored.
 * capture_time is when the window was captured, in the microseconds that
 * baton_stream_source_clock counts. A sample of more than 8 MiB is not read.
 *
 * geometry is how the window was laid out in this picture. A struct that ends before it, as one
 * did before input was carried, is read with a geometry of zero.
 */
typedef struct BatonPicture {
    uint32_t size;
    uint32_t flags;
    uint64_t capture_time;
    uint32_t length_size;
    uint32_t reserved;
    BatonBytes sample;
    const BatonBytes *sets;  /* set_count entries */
    size_t set_count;
    BatonGeometry geometry;
} BatonPicture;

/*
 * A window that is in front of the shared one: where it is, and what the Rust side needs to know
 * of it. flags holds BATON_FRONT_ bits.
 */
typedef struct BatonFront {
    BatonRect rect;
    uint32_t flags;
    uint32_t reserved;
} BatonFront;

/*
 * What the Mac is doing around the window at the moment of the call, for the events that the
 * person at the other device sent. The Rust side writes size, front_cap and fronts, and sets
 * locked to 1, on_screen to 0 and keyboard to BATON_KEYBOARD_UNKNOWN, so that what the platform
 * leaves unwritten reads as the careful answer. The platform writes the rest:
 *
 *   locked:      1 if the Mac is locked or its display is off, 0 if not.
 *   on_screen:   1 if the shared window is on screen, 0 if it is minimized, closed or gone.
 *   keyboard:    BATON_KEYBOARD_SHARED if the application of the window is in front and the
 *                window is its focused window, BATON_KEYBOARD_ELSEWHERE if another window has
 *                the keyboard, BATON_KEYBOARD_UNKNOWN if it cannot tell.
 *   window:      where the shared window is now.
 *   fronts:      room for front_cap windows, written by the Rust side. The platform writes the
 *                windows that lie in front of the shared window and draw something, in any order,
 *                and leaves out one that is fully transparent, or on a layer that covers nothing
 *                and takes no keys.
 *   front_count: how many of them the platform wrote, at most front_cap. A platform that has
 *                more fails the call.
 *
 * For a whole display (BATON_PICKED_DISPLAY) window is the display's own rectangle on the screen,
 * in points, and on_screen is 1 while the display is there. Everything on a display is part of
 * it, so the Rust side takes no notice of keyboard and fronts for one: the keys go to whatever has
 * the keyboard, and nothing lies in front.
 */
typedef struct BatonSurroundings {
    uint32_t size;
    uint32_t locked;
    uint32_t on_screen;
    uint32_t keyboard;
    BatonRect window;
    BatonFront *fronts;
    uint32_t front_cap;
    uint32_t front_count;
} BatonSurroundings;

/*
 * One thing that the person did at the other device, put to the shared window. The Rust side has
 * decided that it belongs to this window and made its place a place on the screen. Fields that
 * the kind does not use are zero.
 *
 *   BATON_POST_POINTER: a pointer at x and y, in points on the screen. action says what it did
 *                       (BATON_POINTER_), and button which button went down or came up. A scroll
 *                       has dx and dy in points: a positive dx moves the content to the right and
 *                       a positive dy moves it down, as a finger that drags the content does,
 *                       whatever the platform's own calls count as positive.
 *   BATON_POST_KEY:     a key. code is a USB HID usage of the keyboard page (0x07), where a is
 *                       0x04, or 0 for text that came with no key: that is typed, goes down and
 *                       up once, and is not held. down is 1 when the key went down and 0 when it
 *                       came up, repeat is 1 when the system repeats a key that is held, and
 *                       text is what the key produced, if anything (UTF-8, at most 20 UTF-16
 *                       units). Typed text is a secret: do not log it.
 *
 * modifiers holds BATON_MOD_ bits: the modifier keys that are down. The platform puts them on the
 * event it makes.
 */
typedef struct BatonPost {
    uint32_t size;
    uint32_t kind;
    double x;
    double y;
    uint32_t action;
    uint32_t button;
    float dx;
    float dy;
    uint32_t modifiers;
    uint32_t code;
    uint32_t down;
    uint32_t repeat;
    BatonBytes text;
} BatonPost;

/*
 * What the packets of sound are: the platform that sends sound gives it to
 * baton_stream_source_sound_begin before the first packet of a run, and the Rust side gives it to
 * sound before the first packet that a screen plays. A session has one form for the whole of it.
 *
 * codec is BATON_SOUND_CODEC_AAC. rate is the sampling rate in hertz, from 8000 to 96000, and
 * channels is 1 or 2. samples is how many samples of each channel one packet holds, from 256 to
 * 2048. A viewer plays AAC-LC with 1024 samples in a packet, or 960 in the variant that some
 * encoders make, and no other form: a sender that begins another is not heard, and is told with
 * hear that the sound is not wanted. The Rust side calls sound with a form that the viewer plays.
 */
typedef struct BatonSound {
    uint32_t size;
    uint32_t codec;
    uint32_t rate;
    uint32_t channels;
    uint32_t samples;
    uint32_t reserved;
} BatonSound;

/*
 * A packet of sound for the screen to decode and play, in the order in which packets are to be
 * played. packet is the bytes of the packet in the codec of the BatonSound before it; for AAC-LC
 * one access unit with no ADTS header and no other framing. flags holds BATON_PLAY_ bits.
 *
 * number counts the packets of a run, and the packets come in the order of their numbers.
 * lost_before is how many packets before this one are missing and are not played if they come
 * later: the screen covers a gap of that many packets, with silence, say. It is 0 for the first
 * packet of a run, which has BATON_PLAY_FIRST. capture_time is when the first sample of the
 * packet was captured, in microseconds since the session began, on the clock of the source.
 */
typedef struct BatonPlay {
    uint32_t size;
    uint32_t flags;
    uint64_t number;
    uint64_t capture_time;
    uint64_t lost_before;
    BatonBytes packet;
} BatonPlay;

/*
 * A packet of sound that the encoder made, as baton_stream_source_sound gives it. bytes is the
 * packet in the codec of the BatonSound that began the run: at least one byte, and a packet of
 * more than 8 KiB is not read. capture_time is when the first sample of the packet was recorded,
 * in the microseconds that baton_stream_source_clock counts, so that sound and pictures have one
 * clock.
 */
typedef struct BatonSoundPacket {
    uint32_t size;
    uint32_t reserved;
    uint64_t capture_time;
    BatonBytes bytes;
} BatonSoundPacket;

/*
 * The functions of the platform. All of them are mandatory but the last three. A function that
 * returns int32_t returns BATON_OK, or something else for a failure.
 *
 *   limits:  fills in what the device can show. Called once, at registration.
 *   open:    the person said yes to a window: makes the decoder and the screen for it, and fills
 *            in screen, whose context is not NULL. link is the number to give
 *            baton_stream_link_decode_failed and baton_stream_link_sound_failed for this screen,
 *            and it is valid until release. On a failure fills in message if it can; a success
 *            without a context is a failure too. Does not wait: a platform that needs time
 *            fails here, with a sentence.
 *   decode:  decodes the frame. A failure means that this frame cannot be decoded, and the Rust
 *            side asks the source for another key frame.
 *   show:    shows the picture of frame number, the newest that decode took.
 *   source:  the source said something about the pictures.
 *   state:   fills in how the decoder is doing.
 *   ended:   the session is over. Let go of what the screen shows; no call follows except release.
 *   release: the screen is not needed any more, whether or not ended came first. Let go of all of
 *            it. Called exactly once, and nothing follows. The link is not valid after it.
 *
 * The last three are about the sound of a screen, and a platform that plays no sound leaves all
 * three NULL: a table that ends at release is accepted, and so is one that gives some of the
 * three and not all, which reads as none. They are called after the person at this device turned
 * the sound on with the app, one after the other, as the calls above are. Only a screen that
 * shows a whole display gets sound and play: the app does not offer the sound of a window, so
 * none of the three is called for a screen that shows one.
 *
 *   sound:   the source sends sound from now on, in this form (BatonSound), which is one that the
 *            screen plays: a run of sound begins. Make a decoder for the form and get ready to
 *            play. The packets that follow are in this form until the next sound or stopped, and
 *            a session has one form for the whole of it.
 *   play:    decodes and plays one packet (BatonPlay), in the order of the numbers: the Rust
 *            side holds a packet that comes early. A packet that cannot be decoded is dropped by
 *            the platform; there is no failure to report, since the Rust side has nothing to do
 *            about it.
 *   stopped: no sound is wanted or sent. why is a BATON_SILENT_ number: BATON_SILENT_OFF if the
 *            person turned the sound off, or if the screen said with
 *            baton_stream_link_sound_failed that it cannot play (the library has turned the sound
 *            off then, and the person has to turn it on again to try again), and otherwise the
 *            reason that the source gave. A form that the library does not play (it plays AAC-LC
 *            in packets of 1024 or 960 samples)
 *            never reaches sound: the library says BATON_SILENT_OTHER itself and turns the sound
 *            off, so the person has to turn it on again. BATON_SILENT_WINDOW, the reason for a
 *            screen that shows a window, is not given: the app does not ask for the sound of
 *            one. No packet follows until the next sound. Let go of the decoder and stop
 *            playing; what is queued need not be played out. It can come when no sound did, and
 *            then there is nothing to let go of. A run that is going when the session ends is
 *            not stopped: ended ends it.
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
typedef void (*BatonSoundFn)(void *screen, const BatonSound *sound);
typedef void (*BatonPlayFn)(void *screen, const BatonPlay *play);
typedef void (*BatonStoppedFn)(void *screen, uint32_t why);

/*
 * What the platform registers: the version of the interface it was written for, the size of this
 * struct as the platform knows it, the pointer that limits and open get as their first argument,
 * and the functions. A struct that is larger than the Rust side knows is accepted, and the end of
 * it is ignored. A table of the size it had before sound was carried (80 bytes, ending at
 * release) is accepted: the three functions it lacks read as NULL.
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
    BatonSoundFn sound;
    BatonPlayFn play;
    BatonStoppedFn stopped;
} BatonViewerVTable;

/*
 * The functions of the platform that shares windows. All of them are mandatory but the last
 * six. The first three are about the device and take the context of the table; the others are
 * about one window, and take the pointer that baton_stream_pick_done gave. BatonEndedFn and
 * BatonReleaseFn are the types of ended and release above, with the pointer of the window in
 * place of the one of the screen.
 *
 *   control:     whether the person can let another device control a window, or a whole display,
 *                that they share, 1 or 0. A platform that cannot post input says 0, and nothing
 *                is offered with control; the same holds for a platform that gave no surroundings
 *                and post, whatever control says. Called each time the app asks what the device
 *                can do.
 *   pick:        the person wants to share a window or a whole display: shows the picker of the
 *                system. request is the number that baton_stream_pick_done is given when the
 *                person has chosen, or given up, or the picker failed. Does not wait for the
 *                person. On a failure to show the picker fills in message if it can, as open
 *                does.
 *   cancel_pick: nobody waits for the answer any more: takes the picker down if it is up. An
 *                answer that comes anyway is refused with BATON_E_UNKNOWN_LINK, and the window
 *                it names stays with the platform.
 *   prepare:     the other device said yes, and takes is what it can take. Chooses the picture to
 *                begin with, within that, fills in format, and gets ready to make it. A failure
 *                means that no picture within what it takes can be made, and the person is told
 *                so. Called at most once for a window.
 *   start:       the session runs, and link is its number for the baton_stream_source_
 *                functions: valid until the session is over, and a number that is not used again.
 *                Starts to capture the window and to encode it, as the rules of the pictures
 *                below say. Does not wait for the first picture. A failure ends the session, and
 *                is shown to the person with the sentence in message, if it wrote one (as open
 *                does); nothing follows it but release.
 *   ended:       the session is over, whoever ended it: the other device, the connection, or the
 *                platform with baton_stream_source_end. It is called from a thread of the Rust
 *                side a moment after the end, with no lock held, so it may wait for the capture
 *                threads of the platform to leave the functions of this interface: the link is
 *                closed before ended is called, and a call that is on its way is answered with
 *                BATON_E_UNKNOWN_LINK. Stop capturing and encoding, the sound included, and let
 *                go of what the capture holds. Called only after a start that returned BATON_OK,
 *                once, and also when the runtime of the app is shut down before the session is
 *                over: the link is closed before ended then too. No call follows but release. A
 *                private session that the person at the other device ended by choosing that
 *                this device stay as it is ends with leave, below, in place of ended.
 *   release:     the window is not needed any more, whether or not ended came first, and
 *                whether or not it was ever started. Let go of all of it, the picker's
 *                result included. Called exactly once, and nothing follows.
 *
 * The last two are about the input of a window that was offered with control, and a platform that
 * cannot post input leaves both NULL: a table that ends at release is accepted, and so is one that
 * gives only one of the two, which reads as neither. They are called after a start that returned
 * BATON_OK and not after ended, from the one thread of the Rust side that also calls start, ended
 * and release, one call after the other, so a slow call holds up the events behind it and
 * nothing else of the window.
 *
 *   surroundings: fills in what the Mac is doing around the window, once for each batch of events
 *                that the person at the other device sent (BatonSurroundings). A failure drops
 *                the events of that batch, except the release of a press that was put to the
 *                window.
 *   post:        puts one thing that the person did to the window (BatonPost). A failure drops
 *                that event. A post goes to the system, as if the person at this Mac had done
 *                it, for a window as for a display, and reaches whatever is at that place or has
 *                the keyboard, which the Rust side has judged to be the shared window or display
 *                when it hands the post over. A platform may fail the post of a key that the
 *                system keeps for itself, which would act on more than the shared window. A
 *                release should not fail, since the Rust side forgets the press whether or not
 *                the release was posted, and does not try again.
 *
 * The last one is about the device again, and takes the context of the table. A platform that
 * cannot give its screen without a picker leaves it NULL (a table that ends at post is accepted),
 * and the device does not tell the devices it is paired with that they can ask for its screen.
 *
 *   screen:      a device that the person at this one allowed beforehand asked to see this
 *                screen, so the person is not asked. Gives the main display, as the answer to a
 *                pick would, but now and from the same call: fills in picked and returns
 *                BATON_OK, with nobody choosing anything. picked comes with its size set and
 *                everything else zero. The platform writes width, height, kind
 *                (BATON_PICKED_DISPLAY) and context, as baton_stream_pick_done is given them,
 *                and leaves title and app empty: a display given this way is not named, and the
 *                Rust side reads no bytes of the platform's. The display is released once, with
 *                release, like a window, also when it is never used. An answer that is not whole
 *                (a kind that is not a display, a size of zero) is released the same way, and
 *                counts as a failure. The call may wait for the system, for a few seconds at
 *                most, and is made from a thread of the Rust side that nothing else waits for.
 *                It returns BATON_SCREEN_NEEDS_PERMISSION if the system has not let the program
 *                capture the screen (the person has to allow it, and may have to open the
 *                program again), BATON_SCREEN_UNAVAILABLE if there is no display to give now
 *                (the Mac has none on), or another value for a failure, with the sentence in
 *                message if the platform can write one. Nothing is given then, and nothing is
 *                released.
 *
 *                flags is 0, or BATON_SCREEN_PRIVATE: the device that asked will also control
 *                the screen, and nobody at this device may see what happens on it or use the
 *                device meanwhile. The platform checks beforehand that it can promise that, and
 *                if it cannot it gives nothing: it returns BATON_SCREEN_NEEDS_INPUT_PERMISSION
 *                if the system has not let it hold the keyboard and the pointer, or another
 *                failure. It never gives the display open in its place. The promise holds from
 *                start until the session is over: the screen of the device stays dark, its
 *                keyboard and its pointer stay out of anyone's hands but the session's, and when
 *                the session is over, whatever the cause, the device is locked again, except at
 *                the end that the person at the other device chose to leave the device as it is
 *                (leave, below). A display
 *                that is released without start changes nothing at the device. A flag that the
 *                platform does not know makes the call fail.
 *
 * The next one is about a screen that was given with BATON_SCREEN_PRIVATE, and takes the pointer
 * of the window. A platform that cannot let the device that controls the screen lift what keeps
 * it private leaves it NULL (a table that ends at screen is accepted): the screen then stays as
 * the promise of screen says for the whole session, and the device that controls it is not told
 * that it can lift anything.
 *
 *   lift:        the person at the device that controls the screen switched what keeps this
 *                device private. lifted is all that is lifted now, a sum of BATON_LIFT_SCREEN
 *                (what is on the screens may be seen at this device) and BATON_LIFT_INPUT (the
 *                keyboard and the pointer of this device may be used at it), and each call
 *                replaces the one before. A session starts with 0, which is the promise of
 *                screen, and the person may switch either bit on and off as often as they like:
 *                the platform puts a bit back as readily as it lifts it. Called after a start
 *                that returned BATON_OK and not after ended, from the one thread that also calls
 *                start, ended, release, surroundings and post, and ahead of the events that wait
 *                for that thread. Calls that wait together are folded into the last, so a
 *                platform that was slow to answer hears how things stand and not each step on
 *                the way. A bit that is not listed above is never set.
 *
 *                Nothing else of the promise of screen changes. While BATON_LIFT_INPUT is lifted
 *                the platform does not take what is typed or moved at this device for a reason
 *                to end the session, since the person at the other device let it be used. A
 *                session that ends, whatever the cause and whatever is lifted, puts everything
 *                back and then locks the device, except at the end that the person chose to
 *                leave the device as it is (leave, below). A lift that cannot be put back is not
 *                reported here: the platform ends the session with baton_stream_source_end, from
 *                a thread of its own, and the device is locked.
 *
 * The next one is about the sound of a display, and takes the pointer of the window like the
 * ones about one window do. A platform that cannot send sound leaves it NULL (a table that ends at
 * lift is accepted), and the device does not tell the devices it is paired with that it sends
 * sound. It is called after a start that returned BATON_OK and not after ended, from the one
 * thread that also calls start, ended, release, surroundings, post and lift, in turn with them,
 * and only for a window that is a whole display, once the person at the other device has turned
 * the sound on.
 *
 *   hear:        whether the other device wants to hear the sound of this one, 1 or 0.
 *
 *                1: record the sound of the device and encode it, as AAC-LC (BatonSound says in
 *                what form), and give it to the Rust side. The platform says what its packets
 *                are with baton_stream_source_sound_begin as soon as the encoder knows, and then
 *                gives each packet to baton_stream_source_sound, in the order in which they were
 *                recorded. A platform that cannot record the sound says why with
 *                baton_stream_source_sound_silent instead. Every 1 is answered so, also when the
 *                platform records already: the packets are dropped (BATON_SOUND_DROPPED) until
 *                the begin that answers the 1, and the other device starts a new run at it. A 1
 *                comes again, with no 0 between, when the person asks anew while no sound is
 *                being sent, as the person does to try again after a silence.
 *
 *                0: the other device does not want to hear any more: the person turned the sound
 *                off, the device has gone quiet, or it cannot play the form. Stop recording and
 *                encoding, and let go of what the capture holds; a 0 that finds nothing recorded
 *                changes nothing. The packets that are in flight are not needed, and a packet
 *                that comes after this one is answered BATON_SOUND_DROPPED.
 *
 *                The Rust side tells the latest word and not the ones between: a person who turns
 *                the sound on, off and on again faster than hear answers is told a 1, and one who
 *                turns it on and off, a 0.
 *
 * The very last is about the end of a screen that was given with BATON_SCREEN_PRIVATE, and takes
 * the pointer of the window. A platform that cannot end the privacy of a screen without locking
 * the device leaves it NULL (a table that ends at hear is accepted), and the device does not tell
 * the devices it is paired with that they can ask it to stay as it is: every end locks the device.
 *
 *   leave:       the session is over, and the person at the device that controlled the screen
 *                chose, when they ended it, that this device stay as it is. It is ended with one
 *                difference: the device is not locked. Called in place of ended and never as
 *                well as it, for a window that was given with BATON_SCREEN_PRIVATE and for no
 *                other, after a start that returned BATON_OK, once, and nothing follows but
 *                release. The thread and the rules of ended hold: no lock is held, the link is
 *                closed already, a call of the platform that is on its way is answered with
 *                BATON_E_UNKNOWN_LINK, and the capture threads of the platform may be waited for.
 *
 *                Put everything back that the promise of screen took, as ended does before it
 *                locks: what was lifted is put back, the screens light, the keyboard and the
 *                pointer of the device are given back, the capture of the sound is let go of, and
 *                whatever else the privacy changed is as it was. Then do not lock the device. Do
 *                not wake it or unlock it either: a device that is locked when the session ends
 *                stays as it is. A thing that cannot be put back is a reason to lock after all,
 *                as it is at every end, since the device is not left open because something in
 *                the end went wrong. The Rust side tells the platform to leave only when the
 *                person chose it and the end that followed was the one they chose; every other
 *                end, whatever the cause, is ended, and the device is locked.
 */
typedef uint32_t (*BatonControlFn)(void *context);
typedef int32_t (*BatonPickFn)(void *context, uint64_t request, BatonMessage *message);
typedef void (*BatonCancelPickFn)(void *context, uint64_t request);
typedef int32_t (*BatonPrepareFn)(void *picked, const BatonTakes *takes, BatonFormat *format);
typedef int32_t (*BatonStartFn)(void *picked, uint64_t link, BatonMessage *message);
typedef int32_t (*BatonSurroundingsFn)(void *picked, BatonSurroundings *surroundings);
typedef int32_t (*BatonPostFn)(void *picked, const BatonPost *post);
typedef int32_t (*BatonScreenFn)(void *context, uint32_t flags, BatonPicked *picked,
                                 BatonMessage *message);
typedef void (*BatonLiftFn)(void *picked, uint32_t lifted);
typedef void (*BatonHearFn)(void *picked, uint32_t on);
typedef void (*BatonLeaveFn)(void *picked);

/*
 * What the platform registers to share windows, as the table above does to show them. A table of
 * the size it had before input was carried (72 bytes, ending at release) is accepted: the six
 * functions it lacks read as NULL. So is one of the size it had before the screen could be given
 * without a picker (88 bytes, ending at post): the four functions it lacks read as NULL. So is
 * one of the size it had before the privacy of a screen could be lifted (96 bytes, ending at
 * screen): the three functions it lacks read as NULL. So is one of the size it had before sound
 * was carried (104 bytes, ending at lift): the two functions it lacks read as NULL. And so is one
 * of the size it had before a private session could end without the lock (112 bytes, ending at
 * hear): the function it lacks reads as NULL.
 */
typedef struct BatonSharerVTable {
    uint32_t version;
    uint32_t size;
    void *context;
    BatonControlFn control;
    BatonPickFn pick;
    BatonCancelPickFn cancel_pick;
    BatonPrepareFn prepare;
    BatonStartFn start;
    BatonEndedFn ended;
    BatonReleaseFn release;
    BatonSurroundingsFn surroundings;
    BatonPostFn post;
    BatonScreenFn screen;
    BatonLiftFn lift;
    BatonHearFn hear;
    BatonLeaveFn leave;
} BatonSharerVTable;

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
 * The speaker of a screen could not play the sound: after the library began a run with sound, what
 * decodes the packets or plays them could not be made or started (a call holds the audio of a
 * phone, there is nothing to play on, the system has no decoder for the form). Called from a thread
 * of its own, never from inside a call of the vtable, with the link that open gave for the screen.
 * The library turns the sound of the screen off and tells the other device to stop sending it, so
 * that the other device plays its own sound again, and the person is told that this device cannot
 * play the sound. They try again by turning the sound on, which begins a new run (sound). The
 * platform is not to try again by itself. stopped follows with BATON_SILENT_OFF, and the packets
 * that were on their way are dropped. A report that comes when the sound is off already, as when
 * the person turned it off while the platform tried, is about a run that is over, and changes
 * nothing. Returns BATON_OK, also when the session is not running yet or is over (nothing happens
 * then), or BATON_E_UNKNOWN_LINK, BATON_E_REENTRANT or BATON_E_PANIC.
 */
int32_t baton_stream_link_sound_failed(uint64_t link);

/*
 * Registers the platform's sharer. Call it once, at launch, before the app starts. A platform
 * registers a viewer, a sharer or both: a device that has registered a sharer lets the person
 * share a window, and a device that has not shares none (an iPhone registers only a viewer). The
 * vtable is copied; the platform keeps whatever its context points to for as long as the process
 * lives. Returns BATON_OK, or BATON_E_ARGUMENT (the vtable is wrong), BATON_E_REGISTERED,
 * BATON_E_REENTRANT or BATON_E_PANIC. A registration that fails leaves nothing registered.
 */
int32_t baton_stream_register_sharer(const BatonSharerVTable *vtable);

/*
 * The answer to a pick, once for each request, from any thread but not from inside a call of the
 * vtable (pick that cannot show the picker fails instead). One of three:
 *
 *   - picked is not NULL and failure is NULL: the person chose a window or a display. On
 *     BATON_OK the Rust side owns picked->context and calls release for it exactly once. On any
 *     other result the window stays with the platform, which lets go of it itself.
 *   - picked is NULL and failure holds a sentence: the picker failed. The person is told it, so
 *     it says what happened in plain words (at most 256 bytes, cut to one line).
 *   - picked is NULL, and failure is NULL or empty: the person gave up. Nothing is shown.
 *
 * Returns BATON_OK, or BATON_E_UNKNOWN_LINK (nobody waits for this request: it was never given,
 * it was answered, or cancel_pick took it down), BATON_E_ARGUMENT (a pointer, a size or a value is
 * wrong; the person is waiting, so the request is answered with a failure all the same),
 * BATON_E_REENTRANT or BATON_E_PANIC.
 *
 * The window can be released inside this call. When the app stops waiting for the answer just as
 * it arrives, the answer is taken and the window is let go of at once, on the thread that called
 * this. So release must not take a lock that is held around this call, and must not wait for the
 * thread that is in this call, the main thread included.
 */
int32_t baton_stream_pick_done(uint64_t request, const BatonPicked *picked,
                               const BatonBytes *failure);

/*
 * The functions the platform calls about a window that it shares, with the link that start gave.
 * They may be called from any thread, also at the same time, and none waits for long. Not from
 * inside a call of the vtable: BATON_E_REENTRANT. A link that is not open, because the session
 * is over or the number was never given, is answered with BATON_E_UNKNOWN_LINK, which a capture
 * thread takes as the end and stops at. The link closes a moment after the end, and a call that
 * comes between the two is answered as the session would have: begin says BATON_BEGIN_NOT_NOW,
 * sent says BATON_SENT_OVER, sound says BATON_SOUND_OVER, and the others return BATON_OK and
 * change nothing.
 *
 * The pictures. For each picture that it could make, the platform asks baton_stream_source_begin.
 * BATON_BEGIN_NOT_NOW means that none is wanted now, because too many are in flight, or the
 * session is paused, or the viewer has gone quiet, and the platform skips the picture. Nothing
 * says when that stops holding, and a window that has stopped changing delivers no newer picture,
 * so the platform asks again on its own frame clock, with the newest picture that the capture
 * hands over when it is asked for one, for as long as the window has changed since the last
 * picture it gave to baton_stream_source_sent, or baton_stream_source_recovery_pending says that
 * a picture is wanted: the first picture of a session is, and so is the one after a frame that
 * was given up on. Any other answer is a kind of picture to make: BATON_BEGIN_KEY, a key frame
 * (an IDR picture, from which a decoder can start); BATON_BEGIN_DELTA, a frame that follows the
 * one before; BATON_BEGIN_RECOVERY, one that refers to a long-term reference that the viewer
 * holds, which a platform that marks no picture as one never gets. The platform answers it with
 * exactly one of baton_stream_source_sent and baton_stream_source_cancel, before it asks begin
 * again.
 */

/* The microseconds since the session began: the clock of the capture_time of a picture. */
int32_t baton_stream_source_clock(uint64_t link, uint64_t *micros);

/* Asks whether to make a picture, and which kind: *kind is a BATON_BEGIN_ number. */
int32_t baton_stream_source_begin(uint64_t link, uint32_t *kind);

/* The picture that begin announced could not be made. Nothing is sent. */
int32_t baton_stream_source_cancel(uint64_t link);

/* *pending is 1 if the next picture has to be something other than a delta, so one is wanted
   even if the window has not changed, and 0 if not. */
int32_t baton_stream_source_recovery_pending(uint64_t link, uint32_t *pending);

/*
 * The picture that begin announced was made: sends it. Returns BATON_OK, and *outcome says what
 * became of the picture (a BATON_SENT_ number), and *number is the number of its frame if it
 * went out, and 0 if not. A picture that is not sent leaves the frame given up on, and the next
 * begin answers again.
 *
 * Returns BATON_E_ARGUMENT if the picture was not taken, because a pointer, a size or a length is
 * wrong, or the sample is not a picture that can go on the wire: a length that reaches beyond its
 * end, a NAL unit that could be taken for more than one, a key frame that lacks a sequence or a
 * picture parameter set or holds no slice of an IDR picture, or a key frame whose sequence
 * parameter set describes a picture larger than the format that prepare chose or that the last
 * baton_stream_source_new_epoch gave: the viewer refuses what is larger than it was told, and
 * nothing on this device would say why. The frame that begin announced is given up on then too.
 */
int32_t baton_stream_source_sent(uint64_t link, const BatonPicture *picture, uint32_t *outcome,
                                 uint64_t *number);

/*
 * The window changed: the capture the platform made at capture_time, in the microseconds that
 * baton_stream_source_clock counts, shows something other than the one before. Call it for every
 * capture that shows a change, whether or not a picture is made of it. What the person at the
 * other device touched is out of date from the moment the window changes, and not from the moment
 * the picture was captured; a window that does not change has a current picture however old it
 * is, so a tap on a window that is still works. A picture that baton_stream_source_sent takes
 * counts as a change at its capture time as well, so a platform that only calls that is right
 * about the pictures that it sends, and wrong about those that it drops before them.
 */
int32_t baton_stream_source_changed(uint64_t link, uint64_t capture_time);

/*
 * The size of the picture changed, because the window did or the viewer wants less: starts a new
 * epoch, which begins with a key frame, and tells the viewer. The frames in flight are of the
 * old size, and so is a picture that is being made. baton_stream_source_sent drops it
 * (BATON_SENT_STALE), except a key frame whose sequence parameter set describes more than the
 * new size: that one is refused with BATON_E_ARGUMENT, as any key frame larger than the format
 * is, and the frame is given up on. Returns BATON_E_ARGUMENT, and changes nothing, if the format
 * is beyond what the protocol allows or the viewer takes. Once the session is over it returns
 * BATON_OK and changes nothing, since the end can come while the platform is resizing.
 */
int32_t baton_stream_source_new_epoch(uint64_t link, const BatonFormat *format);

/*
 * Stops the pictures for a while, and tells the viewer why: BATON_PAUSE_LOCKED when the Mac is
 * locked or its display is off, BATON_PAUSE_HIDDEN when the window is minimized or not on screen,
 * BATON_PAUSE_OTHER for any other reason. A reason that is none of the three returns
 * BATON_E_ARGUMENT and changes nothing. begin answers BATON_BEGIN_NOT_NOW until
 * baton_stream_source_resume. What the person does at the other device is dropped while the
 * session is paused, and whatever it holds down is released.
 */
int32_t baton_stream_source_pause(uint64_t link, uint32_t reason);

/* The pictures go on. */
int32_t baton_stream_source_resume(uint64_t link);

/*
 * The sound. A platform that has hear sends the sound of a display while hear says 1. Once the
 * encoder knows what its packets are, the platform says so with baton_stream_source_sound_begin,
 * and gives each packet to baton_stream_source_sound as the encoder makes it, at the pace at
 * which the sound is recorded: the Rust side keeps a limited number of packets in flight, and
 * drops the packet that has no room. If the platform cannot send the sound it says why with
 * baton_stream_source_sound_silent. While the other device does not want to hear (before the
 * first hear(1), and after a hear(0)), the begin and the silent do nothing and return BATON_OK,
 * and a packet is answered BATON_SOUND_DROPPED, so a platform that is on its way with a packet
 * when hear(0) comes is not harmed by it.
 */

/*
 * Says what the packets of sound are, before the first packet of a run: a run begins here, for
 * the other device, which is told the form and then plays the packets. The platform gives it
 * again after every hear(1) that it answers with sound, and the form is the same every time in a
 * session: it converts the sound if the device it records changes. Returns BATON_OK, and changes
 * nothing if the other device does not want to hear any more. Returns BATON_E_ARGUMENT, and
 * changes nothing, if sound is not a form (a pointer, a size, a codec or a number that is wrong,
 * or beyond what the protocol allows), or differs from the form of an earlier run of the session.
 */
int32_t baton_stream_source_sound_begin(uint64_t link, const BatonSound *sound);

/*
 * The platform cannot send the sound that is wanted, or can no more: ends the run, if one goes,
 * and tells the other device why, which is one of BATON_SILENT_NO_PERMISSION, BATON_SILENT_FAILED
 * and BATON_SILENT_OTHER; any other reason returns BATON_E_ARGUMENT and changes nothing. The
 * person at the other device can try again, which the platform learns from another hear(1).
 */
int32_t baton_stream_source_sound_silent(uint64_t link, uint32_t reason);

/*
 * A packet of sound was made: sends it. Returns BATON_OK, and *outcome says what became of the
 * packet (a BATON_SOUND_ number), and *number is the number of the packet if it went out, and 0
 * if not. A packet that does not go out is lost, and the other device covers the gap that it
 * leaves.
 *
 * Returns BATON_E_ARGUMENT if the packet was not taken, because a pointer or a size is wrong, or
 * it holds no bytes. A packet of more than 8 KiB is not read: *outcome says BATON_SOUND_TOO_LARGE.
 */
int32_t baton_stream_source_sound(uint64_t link, const BatonSoundPacket *packet, uint32_t *outcome,
                                  uint64_t *number);

/*
 * Ends the session. When sentence is NULL or empty, the person closed the window or ended the
 * session themselves, and nothing is shown. Otherwise something went wrong, such as a capture
 * that cannot begin or that the system stopped, and the sentence says what happened in plain
 * words that the person can act on (at most 256 bytes, cut to one line): the person is shown it.
 * The first sentence counts, and a session that is over already is told as it ended, whatever the
 * sentence says: the other device or the connection may have ended it first, or the platform
 * itself with no sentence. ended is called a moment later, from a thread of the Rust side: never
 * from inside this call, but possibly before it returns, while the thread that called this is
 * still on its way out. Ending a session that is over while its link is still open returns
 * BATON_OK and changes nothing.
 */
int32_t baton_stream_source_end(uint64_t link, const BatonBytes *sentence);

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

BATON_STATIC_ASSERT(sizeof(BatonTakes) == 24, "BatonTakes");
BATON_STATIC_ASSERT(offsetof(BatonTakes, size) == 0, "BatonTakes.size");
BATON_STATIC_ASSERT(offsetof(BatonTakes, max_width) == 4, "BatonTakes.max_width");
BATON_STATIC_ASSERT(offsetof(BatonTakes, max_height) == 8, "BatonTakes.max_height");
BATON_STATIC_ASSERT(offsetof(BatonTakes, max_fps) == 12, "BatonTakes.max_fps");
BATON_STATIC_ASSERT(offsetof(BatonTakes, codecs) == 16, "BatonTakes.codecs");
BATON_STATIC_ASSERT(offsetof(BatonTakes, reserved) == 20, "BatonTakes.reserved");

BATON_STATIC_ASSERT(sizeof(BatonFormat) == 24, "BatonFormat");
BATON_STATIC_ASSERT(offsetof(BatonFormat, size) == 0, "BatonFormat.size");
BATON_STATIC_ASSERT(offsetof(BatonFormat, codec) == 4, "BatonFormat.codec");
BATON_STATIC_ASSERT(offsetof(BatonFormat, width) == 8, "BatonFormat.width");
BATON_STATIC_ASSERT(offsetof(BatonFormat, height) == 12, "BatonFormat.height");
BATON_STATIC_ASSERT(offsetof(BatonFormat, fps) == 16, "BatonFormat.fps");
BATON_STATIC_ASSERT(offsetof(BatonFormat, reserved) == 20, "BatonFormat.reserved");

BATON_STATIC_ASSERT(sizeof(BatonPicked) == 56, "BatonPicked");
BATON_STATIC_ASSERT(offsetof(BatonPicked, size) == 0, "BatonPicked.size");
BATON_STATIC_ASSERT(offsetof(BatonPicked, width) == 4, "BatonPicked.width");
BATON_STATIC_ASSERT(offsetof(BatonPicked, height) == 8, "BatonPicked.height");
BATON_STATIC_ASSERT(offsetof(BatonPicked, kind) == 12, "BatonPicked.kind");
BATON_STATIC_ASSERT(offsetof(BatonPicked, context) == 16, "BatonPicked.context");
BATON_STATIC_ASSERT(offsetof(BatonPicked, title) == 24, "BatonPicked.title");
BATON_STATIC_ASSERT(offsetof(BatonPicked, app) == 40, "BatonPicked.app");

BATON_STATIC_ASSERT(sizeof(BatonRect) == 32, "BatonRect");
BATON_STATIC_ASSERT(offsetof(BatonRect, x) == 0, "BatonRect.x");
BATON_STATIC_ASSERT(offsetof(BatonRect, y) == 8, "BatonRect.y");
BATON_STATIC_ASSERT(offsetof(BatonRect, width) == 16, "BatonRect.width");
BATON_STATIC_ASSERT(offsetof(BatonRect, height) == 24, "BatonRect.height");

BATON_STATIC_ASSERT(sizeof(BatonGeometry) == 56, "BatonGeometry");
BATON_STATIC_ASSERT(offsetof(BatonGeometry, window_width) == 0, "BatonGeometry.window_width");
BATON_STATIC_ASSERT(offsetof(BatonGeometry, window_height) == 8, "BatonGeometry.window_height");
BATON_STATIC_ASSERT(offsetof(BatonGeometry, picture_width) == 16, "BatonGeometry.picture_width");
BATON_STATIC_ASSERT(offsetof(BatonGeometry, picture_height) == 20,
                    "BatonGeometry.picture_height");
BATON_STATIC_ASSERT(offsetof(BatonGeometry, content) == 24, "BatonGeometry.content");

BATON_STATIC_ASSERT(sizeof(BatonPicture) == 112, "BatonPicture");
BATON_STATIC_ASSERT(offsetof(BatonPicture, size) == 0, "BatonPicture.size");
BATON_STATIC_ASSERT(offsetof(BatonPicture, flags) == 4, "BatonPicture.flags");
BATON_STATIC_ASSERT(offsetof(BatonPicture, capture_time) == 8, "BatonPicture.capture_time");
BATON_STATIC_ASSERT(offsetof(BatonPicture, length_size) == 16, "BatonPicture.length_size");
BATON_STATIC_ASSERT(offsetof(BatonPicture, reserved) == 20, "BatonPicture.reserved");
BATON_STATIC_ASSERT(offsetof(BatonPicture, sample) == 24, "BatonPicture.sample");
BATON_STATIC_ASSERT(offsetof(BatonPicture, sets) == 40, "BatonPicture.sets");
BATON_STATIC_ASSERT(offsetof(BatonPicture, set_count) == 48, "BatonPicture.set_count");
BATON_STATIC_ASSERT(offsetof(BatonPicture, geometry) == 56, "BatonPicture.geometry");

BATON_STATIC_ASSERT(sizeof(BatonFront) == 40, "BatonFront");
BATON_STATIC_ASSERT(offsetof(BatonFront, rect) == 0, "BatonFront.rect");
BATON_STATIC_ASSERT(offsetof(BatonFront, flags) == 32, "BatonFront.flags");
BATON_STATIC_ASSERT(offsetof(BatonFront, reserved) == 36, "BatonFront.reserved");

BATON_STATIC_ASSERT(sizeof(BatonSurroundings) == 64, "BatonSurroundings");
BATON_STATIC_ASSERT(offsetof(BatonSurroundings, size) == 0, "BatonSurroundings.size");
BATON_STATIC_ASSERT(offsetof(BatonSurroundings, locked) == 4, "BatonSurroundings.locked");
BATON_STATIC_ASSERT(offsetof(BatonSurroundings, on_screen) == 8, "BatonSurroundings.on_screen");
BATON_STATIC_ASSERT(offsetof(BatonSurroundings, keyboard) == 12, "BatonSurroundings.keyboard");
BATON_STATIC_ASSERT(offsetof(BatonSurroundings, window) == 16, "BatonSurroundings.window");
BATON_STATIC_ASSERT(offsetof(BatonSurroundings, fronts) == 48, "BatonSurroundings.fronts");
BATON_STATIC_ASSERT(offsetof(BatonSurroundings, front_cap) == 56, "BatonSurroundings.front_cap");
BATON_STATIC_ASSERT(offsetof(BatonSurroundings, front_count) == 60,
                    "BatonSurroundings.front_count");

BATON_STATIC_ASSERT(sizeof(BatonPost) == 72, "BatonPost");
BATON_STATIC_ASSERT(offsetof(BatonPost, size) == 0, "BatonPost.size");
BATON_STATIC_ASSERT(offsetof(BatonPost, kind) == 4, "BatonPost.kind");
BATON_STATIC_ASSERT(offsetof(BatonPost, x) == 8, "BatonPost.x");
BATON_STATIC_ASSERT(offsetof(BatonPost, y) == 16, "BatonPost.y");
BATON_STATIC_ASSERT(offsetof(BatonPost, action) == 24, "BatonPost.action");
BATON_STATIC_ASSERT(offsetof(BatonPost, button) == 28, "BatonPost.button");
BATON_STATIC_ASSERT(offsetof(BatonPost, dx) == 32, "BatonPost.dx");
BATON_STATIC_ASSERT(offsetof(BatonPost, dy) == 36, "BatonPost.dy");
BATON_STATIC_ASSERT(offsetof(BatonPost, modifiers) == 40, "BatonPost.modifiers");
BATON_STATIC_ASSERT(offsetof(BatonPost, code) == 44, "BatonPost.code");
BATON_STATIC_ASSERT(offsetof(BatonPost, down) == 48, "BatonPost.down");
BATON_STATIC_ASSERT(offsetof(BatonPost, repeat) == 52, "BatonPost.repeat");
BATON_STATIC_ASSERT(offsetof(BatonPost, text) == 56, "BatonPost.text");

BATON_STATIC_ASSERT(sizeof(BatonSound) == 24, "BatonSound");
BATON_STATIC_ASSERT(offsetof(BatonSound, size) == 0, "BatonSound.size");
BATON_STATIC_ASSERT(offsetof(BatonSound, codec) == 4, "BatonSound.codec");
BATON_STATIC_ASSERT(offsetof(BatonSound, rate) == 8, "BatonSound.rate");
BATON_STATIC_ASSERT(offsetof(BatonSound, channels) == 12, "BatonSound.channels");
BATON_STATIC_ASSERT(offsetof(BatonSound, samples) == 16, "BatonSound.samples");
BATON_STATIC_ASSERT(offsetof(BatonSound, reserved) == 20, "BatonSound.reserved");

BATON_STATIC_ASSERT(sizeof(BatonPlay) == 48, "BatonPlay");
BATON_STATIC_ASSERT(offsetof(BatonPlay, size) == 0, "BatonPlay.size");
BATON_STATIC_ASSERT(offsetof(BatonPlay, flags) == 4, "BatonPlay.flags");
BATON_STATIC_ASSERT(offsetof(BatonPlay, number) == 8, "BatonPlay.number");
BATON_STATIC_ASSERT(offsetof(BatonPlay, capture_time) == 16, "BatonPlay.capture_time");
BATON_STATIC_ASSERT(offsetof(BatonPlay, lost_before) == 24, "BatonPlay.lost_before");
BATON_STATIC_ASSERT(offsetof(BatonPlay, packet) == 32, "BatonPlay.packet");

BATON_STATIC_ASSERT(sizeof(BatonSoundPacket) == 32, "BatonSoundPacket");
BATON_STATIC_ASSERT(offsetof(BatonSoundPacket, size) == 0, "BatonSoundPacket.size");
BATON_STATIC_ASSERT(offsetof(BatonSoundPacket, reserved) == 4, "BatonSoundPacket.reserved");
BATON_STATIC_ASSERT(offsetof(BatonSoundPacket, capture_time) == 8,
                    "BatonSoundPacket.capture_time");
BATON_STATIC_ASSERT(offsetof(BatonSoundPacket, bytes) == 16, "BatonSoundPacket.bytes");

BATON_STATIC_ASSERT(sizeof(BatonViewerVTable) == 104, "BatonViewerVTable");
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
BATON_STATIC_ASSERT(offsetof(BatonViewerVTable, sound) == 80, "BatonViewerVTable.sound");
BATON_STATIC_ASSERT(offsetof(BatonViewerVTable, play) == 88, "BatonViewerVTable.play");
BATON_STATIC_ASSERT(offsetof(BatonViewerVTable, stopped) == 96, "BatonViewerVTable.stopped");

BATON_STATIC_ASSERT(sizeof(BatonSharerVTable) == 120, "BatonSharerVTable");
BATON_STATIC_ASSERT(offsetof(BatonSharerVTable, version) == 0, "BatonSharerVTable.version");
BATON_STATIC_ASSERT(offsetof(BatonSharerVTable, size) == 4, "BatonSharerVTable.size");
BATON_STATIC_ASSERT(offsetof(BatonSharerVTable, context) == 8, "BatonSharerVTable.context");
BATON_STATIC_ASSERT(offsetof(BatonSharerVTable, control) == 16, "BatonSharerVTable.control");
BATON_STATIC_ASSERT(offsetof(BatonSharerVTable, pick) == 24, "BatonSharerVTable.pick");
BATON_STATIC_ASSERT(offsetof(BatonSharerVTable, cancel_pick) == 32,
                    "BatonSharerVTable.cancel_pick");
BATON_STATIC_ASSERT(offsetof(BatonSharerVTable, prepare) == 40, "BatonSharerVTable.prepare");
BATON_STATIC_ASSERT(offsetof(BatonSharerVTable, start) == 48, "BatonSharerVTable.start");
BATON_STATIC_ASSERT(offsetof(BatonSharerVTable, ended) == 56, "BatonSharerVTable.ended");
BATON_STATIC_ASSERT(offsetof(BatonSharerVTable, release) == 64, "BatonSharerVTable.release");
BATON_STATIC_ASSERT(offsetof(BatonSharerVTable, surroundings) == 72,
                    "BatonSharerVTable.surroundings");
BATON_STATIC_ASSERT(offsetof(BatonSharerVTable, post) == 80, "BatonSharerVTable.post");
BATON_STATIC_ASSERT(offsetof(BatonSharerVTable, screen) == 88, "BatonSharerVTable.screen");
BATON_STATIC_ASSERT(offsetof(BatonSharerVTable, lift) == 96, "BatonSharerVTable.lift");
BATON_STATIC_ASSERT(offsetof(BatonSharerVTable, hear) == 104, "BatonSharerVTable.hear");
BATON_STATIC_ASSERT(offsetof(BatonSharerVTable, leave) == 112, "BatonSharerVTable.leave");

#ifdef __cplusplus
}
#endif

#endif /* BATON_STREAM_ABI_H */
