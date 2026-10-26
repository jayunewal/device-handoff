// SPDX-License-Identifier: Apache-2.0

//! The viewer that the platform registered, as the app's [`WindowViewer`], and the decoder and the
//! speaker of each of its screens, as the session's [`Decoder`] and [`Speaker`].
//!
//! The decoder is where the Rust side stands between the network and the platform. A picture
//! comes in the form of the wire (H.264 in Annex B form) and goes out in the form that the Apple
//! decoder takes (AVCC), with its parameter sets taken out for the description of the format
//! (`baton_stream::h264`). Before the platform sees a parameter set, the size it describes is read
//! and held to what the device said it can show: a decoder allocates by the sets, which come from a
//! paired device that may be broken, and the source's `Config` is checked against the same limits
//! but says nothing about what is inside the pictures.
//!
//! The speaker is the other half of a screen, for a platform that plays sound: it passes the form
//! of the sound, the packets and the end of a run to the platform, in the order that the session
//! gives them. It stands for the same screen as the decoder, so the two share it: whichever of them
//! is dropped last releases the screen, after the link is closed.

use std::sync::{Arc, Mutex};

use baton_app::windows::{PlatformError, Screen, ScreenLink, ShowLimits, WindowViewer};
use baton_core::streaming::{DecodeError, Decoder, DecoderState, Play, Speaker, WindowOffer};
use baton_proto::stream::Codec;
use baton_stream::{
    control::{PauseReason, SilentReason, Sound, SourceMessage, Thermal},
    gate::Reason,
    h264,
    header::FrameHeader,
};

use crate::{
    Error,
    abi::{
        BatonDecoderState, BatonLimits, BatonSourceMessage, CODEC_H264,
        DROPPED_APP_WINDOW_IN_FRONT, DROPPED_CONTROL_OFF, DROPPED_COVERED, DROPPED_FRAME_UNKNOWN,
        DROPPED_LOCKED, DROPPED_MALFORMED, DROPPED_NO_KEYBOARD, DROPPED_NOT_ON_SCREEN,
        DROPPED_NOT_PRESSED, DROPPED_OTHER, DROPPED_OUTSIDE_WINDOW, DROPPED_PAUSED,
        DROPPED_PICTURE_OUT_OF_DATE, DROPPED_TOO_FAST, DROPPED_TOO_MANY_HELD,
        DROPPED_WINDOW_RESIZED, FRAME_KEY, FRAME_RECOVERY, FRAME_REFERENCED, PAUSE_HIDDEN,
        PAUSE_LOCKED, PAUSE_OTHER, SOURCE_CONFIG, SOURCE_NOTICE, SOURCE_PAUSE, SOURCE_RESUME,
        THERMAL_CRITICAL, THERMAL_FAIR, THERMAL_NOMINAL, THERMAL_SERIOUS, size_of_u32,
    },
    platform::{Calls, Frame, ScreenHandle, Window},
    registry::{Links, lock},
};

/// What the person is told when the platform made no screen and gave no reason.
const NO_SCREEN: &str = "This device could not show the window.";

/// The viewer of the platform.
pub(crate) struct AbiViewer {
    calls: Calls,
    limits: ShowLimits,
    links: Arc<Links>,
}

impl AbiViewer {
    /// A viewer for the platform that registered `calls`, which said `limits`.
    ///
    /// # Errors
    ///
    /// The limits name a codec that this version does not know, or a rate that does not fit.
    /// Whether the numbers are within what the protocol allows is for the app to say when it starts.
    pub(crate) fn new(
        calls: Calls,
        limits: &BatonLimits,
        links: Arc<Links>,
    ) -> Result<Self, Error> {
        Ok(Self {
            calls,
            limits: show_limits(limits)?,
            links,
        })
    }
}

/// What the device can show, from what the platform said.
fn show_limits(limits: &BatonLimits) -> Result<ShowLimits, Error> {
    if limits.codecs & !CODEC_H264 != 0 {
        return Err(Error::Argument);
    }
    let max_fps = u16::try_from(limits.max_fps).map_err(|_| Error::Argument)?;
    let mut codecs = Vec::new();
    if limits.codecs & CODEC_H264 != 0 {
        codecs.push(Codec::H264);
    }
    Ok(ShowLimits {
        max_width: limits.max_width,
        max_height: limits.max_height,
        max_fps,
        codecs,
    })
}

impl WindowViewer for AbiViewer {
    fn limits(&self) -> ShowLimits {
        self.limits.clone()
    }

    fn open(&self, window: &WindowOffer, link: ScreenLink) -> Result<Screen, PlatformError> {
        let number = self.links.add(link);
        let opened = self.calls.open(
            &Window {
                title: window.title.as_deref().unwrap_or_default(),
                app: window.app.as_deref().unwrap_or_default(),
                width: window.width,
                height: window.height,
                control: window.control,
            },
            number,
        );
        match opened {
            Ok(opened) => {
                let held = Arc::new(Mutex::new(Held {
                    screen: opened.screen,
                    over: false,
                }));
                let decoder = AbiDecoder {
                    held: Arc::clone(&held),
                    max_width: self.limits.max_width,
                    max_height: self.limits.max_height,
                    link: number,
                    links: Arc::clone(&self.links),
                    sample: Vec::new(),
                };
                let screen = Screen::new(Box::new(decoder), opened.id);
                Ok(if self.calls.can_play() {
                    screen.with_speaker(AbiSpeaker { held })
                } else {
                    screen
                })
            }
            Err(failure) => {
                self.links.remove(number);
                Err(PlatformError::new(
                    failure.0.unwrap_or_else(|| NO_SCREEN.to_owned()),
                ))
            }
        }
    }

    fn plays_sound(&self) -> bool {
        self.calls.can_play()
    }
}

/// The screen of the platform, which the decoder and the speaker of a screen both call.
///
/// The session calls the two with its lock held, one after the other, so the lock around this is
/// never waited for: it only lets the two own the screen together.
struct Held {
    screen: ScreenHandle,
    /// Whether the platform was told that the session is over. Nothing is passed to it after that,
    /// though the session does not call either.
    over: bool,
}

impl Held {
    /// Tells the platform that the session is over, once.
    fn end(&mut self) {
        if !self.over {
            self.over = true;
            self.screen.ended();
        }
    }
}

/// The decoder of one screen.
struct AbiDecoder {
    held: Arc<Mutex<Held>>,
    /// The largest picture the device said it can show.
    max_width: u32,
    max_height: u32,
    /// The number the platform was given to say that the decoder failed.
    link: u64,
    links: Arc<Links>,
    /// Room for the sample of the next frame, kept to save an allocation for each.
    sample: Vec<u8>,
}

impl AbiDecoder {
    /// Whether a sequence parameter set describes a picture that the device said it can show, as it
    /// is shown and as it is coded: a decoder allocates for what is coded, and a set can crop a
    /// large picture down to a small one.
    fn fits(&self, set: &[u8]) -> bool {
        h264::picture_fits(set, self.max_width, self.max_height)
    }
}

impl Drop for AbiDecoder {
    fn drop(&mut self) {
        // The platform's link ends before its screen is released, so a call that is on its way
        // from a thread of the platform finds the link closed and not a screen that is gone. The
        // screen is released when the last of the decoder and the speaker is dropped, which is
        // after this.
        self.links.remove(self.link);
    }
}

impl Decoder for AbiDecoder {
    fn decode(&mut self, frame: &FrameHeader, picture: Vec<u8>) -> Result<(), DecodeError> {
        let mut held = lock(&self.held);
        if held.over {
            return Err(DecodeError);
        }
        self.sample.clear();
        let sets = h264::annex_b_to_avcc(&picture, &mut self.sample).map_err(|_| DecodeError)?;
        // A decoder allocates by the sequence parameter sets, in any frame that carries them.
        if !sets.sps.iter().all(|set| self.fits(set)) {
            return Err(DecodeError);
        }
        // A key frame is the start of a decoder, and cannot be one without the sets that say
        // what the pictures are, or without the picture that is coded in them.
        if frame.is_key() && !(sets.idr && !sets.sps.is_empty() && !sets.pps.is_empty()) {
            return Err(DecodeError);
        }
        let taken = held.screen.decode(&Frame {
            flags: frame_flags(frame),
            number: frame.number,
            depends_on: frame.depends_on,
            capture_time: frame.capture_time,
            chain: frame.chain,
            epoch: frame.epoch,
            sample: &self.sample,
            sequence_sets: &sets.sps,
            picture_sets: &sets.pps,
        });
        if taken { Ok(()) } else { Err(DecodeError) }
    }

    fn show(&mut self, number: u64) {
        let mut held = lock(&self.held);
        if !held.over {
            held.screen.show(number);
        }
    }

    fn source(&mut self, message: SourceMessage) {
        let mut held = lock(&self.held);
        if !held.over
            && let Some(message) = source_message(&message)
        {
            held.screen.source(&message);
        }
    }

    fn state(&self) -> DecoderState {
        let held = lock(&self.held);
        if held.over {
            return DecoderState {
                decode_us: 0,
                thermal: Thermal::Nominal,
            };
        }
        held.screen.state().into()
    }

    fn ended(&mut self) {
        lock(&self.held).end();
    }
}

/// The speaker of one screen, for a platform that plays sound. It is given what the session gives
/// it, in that order, and passes it to the platform's screen; nothing is passed after the end.
struct AbiSpeaker {
    held: Arc<Mutex<Held>>,
}

impl Speaker for AbiSpeaker {
    fn sound(&mut self, sound: Sound) {
        let mut held = lock(&self.held);
        if !held.over {
            held.screen.sound(sound);
        }
    }

    fn play(&mut self, play: Play, packet: Vec<u8>) {
        let mut held = lock(&self.held);
        if !held.over {
            held.screen.play(play, &packet);
        }
    }

    fn stopped(&mut self, why: Option<SilentReason>) {
        let mut held = lock(&self.held);
        if !held.over {
            held.screen.stopped(why);
        }
    }

    /// The session is over for the screen as a whole: whichever of the decoder and the speaker
    /// hears it first tells the platform, and the other finds it told.
    fn ended(&mut self) {
        lock(&self.held).end();
    }
}

/// The `FRAME_` bits of a frame's flags.
fn frame_flags(frame: &FrameHeader) -> u32 {
    use baton_stream::header::Flags;
    let mut bits = 0;
    if frame.flags.contains(Flags::KEY) {
        bits |= FRAME_KEY;
    }
    if frame.flags.contains(Flags::RECOVERY) {
        bits |= FRAME_RECOVERY;
    }
    if frame.flags.contains(Flags::REFERENCED) {
        bits |= FRAME_REFERENCED;
    }
    bits
}

/// How hot the device is, from what the platform says.
fn thermal(state: u32) -> Thermal {
    match state {
        THERMAL_NOMINAL => Thermal::Nominal,
        THERMAL_FAIR => Thermal::Fair,
        THERMAL_SERIOUS => Thermal::Serious,
        THERMAL_CRITICAL => Thermal::Critical,
        // `THERMAL_OTHER`, and any state that a newer platform knows and this version does not.
        _ => Thermal::Other,
    }
}

/// What the source said about the pictures, in the form of the interface. What it said about sound
/// does not go through here: sound reaches the platform by calls of its own, and a session does
/// not hand a viewer that does not take sound a message about it. What it said about an unlock
/// does not either, and a session does not hand a viewer that does not take part in unlocks a
/// message about that.
fn source_message(message: &SourceMessage) -> Option<BatonSourceMessage> {
    let mut out = BatonSourceMessage {
        size: size_of_u32::<BatonSourceMessage>(),
        kind: 0,
        epoch: 0,
        codec: 0,
        width: 0,
        height: 0,
        fps: 0,
        reason: 0,
        count: 0,
        reserved: 0,
    };
    match message {
        SourceMessage::Config(config) => {
            out.kind = SOURCE_CONFIG;
            out.epoch = config.epoch;
            out.codec = match config.codec {
                Codec::H264 => CODEC_H264,
                Codec::Other => 0,
            };
            out.width = config.width;
            out.height = config.height;
            out.fps = u32::from(config.fps);
        }
        SourceMessage::Pause(pause) => {
            out.kind = SOURCE_PAUSE;
            out.reason = match pause.reason {
                PauseReason::Locked => PAUSE_LOCKED,
                PauseReason::Hidden => PAUSE_HIDDEN,
                PauseReason::Other => PAUSE_OTHER,
            };
        }
        SourceMessage::Resume => out.kind = SOURCE_RESUME,
        SourceMessage::Notice(notice) => {
            out.kind = SOURCE_NOTICE;
            out.reason = dropped(notice.reason);
            out.count = notice.count;
        }
        SourceMessage::Sound(_)
        | SourceMessage::Silent(_)
        | SourceMessage::UnlockSealed(_)
        | SourceMessage::UnlockDone(_) => return None,
    }
    Some(out)
}

/// The `DROPPED_` number of a reason for dropping events.
fn dropped(reason: Reason) -> u32 {
    match reason {
        Reason::Malformed => DROPPED_MALFORMED,
        Reason::ControlOff => DROPPED_CONTROL_OFF,
        Reason::Paused => DROPPED_PAUSED,
        Reason::Locked => DROPPED_LOCKED,
        Reason::NotOnScreen => DROPPED_NOT_ON_SCREEN,
        Reason::TooFast => DROPPED_TOO_FAST,
        Reason::FrameUnknown => DROPPED_FRAME_UNKNOWN,
        Reason::PictureOutOfDate => DROPPED_PICTURE_OUT_OF_DATE,
        Reason::WindowResized => DROPPED_WINDOW_RESIZED,
        Reason::OutsideWindow => DROPPED_OUTSIDE_WINDOW,
        Reason::Covered => DROPPED_COVERED,
        Reason::NoKeyboard => DROPPED_NO_KEYBOARD,
        Reason::AppWindowInFront => DROPPED_APP_WINDOW_IN_FRONT,
        Reason::TooManyHeld => DROPPED_TOO_MANY_HELD,
        Reason::NotPressed => DROPPED_NOT_PRESSED,
        Reason::Other => DROPPED_OTHER,
    }
}

impl From<BatonDecoderState> for DecoderState {
    fn from(state: BatonDecoderState) -> Self {
        Self {
            decode_us: state.decode_us,
            thermal: thermal(state.thermal),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use baton_proto::{
        id::SessionId,
        unlock::{Challenge, SealedKey},
    };
    use baton_stream::{
        control::{Config, Pause, Silent, SilentReason, Sound, SoundCodec},
        gate::Notice,
        header::Flags,
        unlock::{Done, Outcome, Sealed},
    };

    use super::*;
    use crate::{
        abi::{
            BatonSound, CODEC_H264, FRAME_KEY, FRAME_REFERENCED, PLAY_FIRST, SILENT_FAILED,
            SILENT_NO_PERMISSION, SILENT_OFF, SILENT_OTHER, SILENT_WINDOW, SOUND_CODEC_AAC,
            SOURCE_CONFIG, SOURCE_NOTICE, SOURCE_PAUSE, SOURCE_RESUME,
        },
        fake::{Call, Fake, Played},
        fixture::{
            HEIGHT, IDR, PPS, SEI, SLICE, SPS, WIDTH, avcc_kinds, kind, pictures, sps, units,
            with_first, without,
        },
        platform::Window,
    };

    fn header(number: u64, flags: Flags) -> FrameHeader {
        FrameHeader {
            session: SessionId::from_bytes([1; 16]),
            number,
            depends_on: number,
            chain: 3,
            epoch: 2,
            capture_time: 16_000 * number,
            flags,
        }
    }

    fn key() -> Flags {
        Flags::KEY | Flags::REFERENCED
    }

    /// A decoder on a screen of the fake platform, with the links it closes.
    struct Rig {
        // The decoder goes first, so that its screen is released while the platform still lives.
        decoder: AbiDecoder,
        links: Arc<Links>,
        fake: Arc<Fake>,
    }

    fn rig(max_width: u32, max_height: u32) -> Rig {
        let fake = Fake::new();
        let calls = fake.platform();
        let window = Window {
            title: "",
            app: "",
            width: WIDTH,
            height: HEIGHT,
            control: false,
        };
        let opened = calls.open(&window, 1).ok().unwrap();
        let links = Arc::new(Links::default());
        let link = links.add(ScreenLink::default());
        let held = Arc::new(Mutex::new(Held {
            screen: opened.screen,
            over: false,
        }));
        let decoder = AbiDecoder {
            held,
            max_width,
            max_height,
            link,
            links: Arc::clone(&links),
            sample: Vec::new(),
        };
        Rig {
            decoder,
            links,
            fake,
        }
    }

    impl Rig {
        /// The speaker of the same screen as the decoder.
        fn speaker(&self) -> AbiSpeaker {
            AbiSpeaker {
                held: Arc::clone(&self.decoder.held),
            }
        }

        fn decoded(&self) -> Vec<crate::fake::Decoded> {
            self.fake
                .calls(1)
                .into_iter()
                .filter_map(|call| match call {
                    Call::Decode(decoded) => Some(decoded),
                    _ => None,
                })
                .collect()
        }
    }

    #[test]
    fn a_key_frame_reaches_the_platform_in_the_form_that_the_apple_decoder_takes() {
        let pictures = pictures();
        assert_eq!(pictures.len(), 3);
        let mut rig = rig(WIDTH, HEIGHT);
        rig.decoder
            .decode(&header(4, key()), pictures[0].clone())
            .unwrap();

        let mut expected = Vec::new();
        let taken = h264::annex_b_to_avcc(&pictures[0], &mut expected).unwrap();
        let sets: Vec<Vec<u8>> = taken.parameter_sets().map(<[u8]>::to_vec).collect();
        let decoded = rig.decoded();
        assert_eq!(decoded.len(), 1);
        let decoded = &decoded[0];
        assert_eq!(decoded.sample, expected);
        // The units for the decoder, behind lengths of four bytes: no delimiter, no parameter
        // set.
        assert_eq!(avcc_kinds(&decoded.sample), [SEI, IDR, IDR]);
        // One sequence set, then one picture set.
        assert_eq!(decoded.sets, sets);
        assert_eq!(decoded.sequence_sets, 1);
        assert_eq!(kind(&decoded.sets[0]), SPS);
        assert_eq!(kind(&decoded.sets[1]), PPS);
        assert_eq!(
            (
                decoded.flags,
                decoded.number,
                decoded.depends_on,
                decoded.capture_time,
                decoded.chain,
                decoded.epoch
            ),
            (FRAME_KEY | FRAME_REFERENCED, 4, 4, 64_000, 3, 2)
        );
        assert!(
            rig.fake.complaints().is_empty(),
            "{:?}",
            rig.fake.complaints()
        );
    }

    #[test]
    fn the_flags_of_a_frame_reach_the_platform_bit_for_bit() {
        use crate::abi::FRAME_RECOVERY;

        let pictures = pictures();
        let mut rig = rig(WIDTH, HEIGHT);
        let cases = [
            (Flags::NONE, 0),
            (Flags::REFERENCED, FRAME_REFERENCED),
            (Flags::RECOVERY, FRAME_RECOVERY),
            (
                Flags::RECOVERY | Flags::REFERENCED,
                FRAME_RECOVERY | FRAME_REFERENCED,
            ),
        ];
        for (number, (flags, _)) in (1..).zip(cases) {
            rig.decoder
                .decode(&header(number, flags), pictures[1].clone())
                .unwrap();
        }
        rig.decoder
            .decode(&header(9, Flags::KEY), pictures[0].clone())
            .unwrap();
        let heard: Vec<u32> = rig.decoded().iter().map(|decoded| decoded.flags).collect();
        let expected: Vec<u32> = cases
            .iter()
            .map(|(_, bits)| *bits)
            .chain([FRAME_KEY])
            .collect();
        assert_eq!(heard, expected);
        assert!(
            rig.fake.complaints().is_empty(),
            "{:?}",
            rig.fake.complaints()
        );
    }

    #[test]
    fn a_delta_frame_reaches_the_platform_without_parameter_sets() {
        let pictures = pictures();
        let mut rig = rig(WIDTH, HEIGHT);
        let delta = header(1, Flags::NONE);
        rig.decoder.decode(&delta, pictures[1].clone()).unwrap();
        let decoded = rig.decoded();
        assert_eq!(avcc_kinds(&decoded[0].sample), [SLICE, SLICE]);
        assert_eq!(decoded[0].sets.len(), 0);
        assert_eq!((decoded[0].sequence_sets, decoded[0].flags), (0, 0));
    }

    #[test]
    fn a_key_frame_that_a_decoder_cannot_start_from_is_refused() {
        let pictures = pictures();
        let cases = [
            ("a delta picture marked as a key frame", pictures[1].clone()),
            ("no sequence parameter set", without(&pictures[0], &[SPS])),
            ("no picture parameter set", without(&pictures[0], &[PPS])),
            ("no slice of an IDR picture", without(&pictures[0], &[IDR])),
        ];
        for (what, picture) in cases {
            let mut rig = rig(WIDTH, HEIGHT);
            assert!(
                rig.decoder.decode(&header(0, key()), picture).is_err(),
                "{what}"
            );
            assert!(rig.decoded().is_empty(), "{what}: the platform was asked");
            // Nothing is latched: the next key frame is taken.
            rig.decoder
                .decode(&header(1, key()), pictures[0].clone())
                .unwrap_or_else(|_| panic!("{what}: the next key frame was refused"));
        }
    }

    #[test]
    fn a_picture_that_is_not_h264_is_refused() {
        let mut rig = rig(WIDTH, HEIGHT);
        for picture in [
            vec![],
            vec![1, 2, 3],
            vec![0, 0, 0, 1],
            vec![0, 0, 0, 1, 0x80],
        ] {
            assert!(
                rig.decoder
                    .decode(&header(0, Flags::NONE), picture.clone())
                    .is_err(),
                "{picture:?}"
            );
        }
        assert_eq!(rig.decoded().len(), 0);
    }

    #[test]
    fn a_parameter_set_for_a_picture_beyond_the_limits_is_refused() {
        let pictures = pictures();
        // The size is at the limit, and one pixel over it in each direction.
        for (what, width, height, taken) in [
            ("at the limit", WIDTH, HEIGHT, true),
            ("one pixel too wide", WIDTH - 1, HEIGHT, false),
            ("one pixel too high", WIDTH, HEIGHT - 1, false),
            ("far too large", 16, 16, false),
        ] {
            let mut rig = rig(width, height);
            let outcome = rig.decoder.decode(&header(0, key()), pictures[0].clone());
            assert_eq!(outcome.is_ok(), taken, "{what}");
            assert_eq!(rig.decoded().len(), usize::from(taken), "{what}");
        }
    }

    #[test]
    fn a_picture_is_held_to_the_limits_as_it_is_coded_and_not_only_as_it_is_shown() {
        let pictures = pictures();
        let rest = without(&pictures[0], &[SPS]);
        // The sets are given as macroblocks of 16 pixels across and down, and the cropping on the
        // right and at the bottom in units of two pixels.
        for (what, blocks, crop, limits, taken) in [
            (
                "coded and shown at the limit",
                (4, 3),
                (0, 0),
                (64, 48),
                true,
            ),
            // An encoder codes 40 pixels as 48, and what it codes is what a decoder allocates.
            (
                "coded a little beyond what is shown",
                (3, 3),
                (4, 4),
                (40, 40),
                true,
            ),
            (
                "coded twice as wide as it is shown",
                (8, 3),
                (32, 0),
                (64, 48),
                false,
            ),
            (
                "a picture of 3840 by 2160 cropped to 64 by 48",
                (240, 135),
                (1888, 1056),
                (64, 48),
                false,
            ),
        ] {
            let set = sps(blocks, crop);
            // What the set says is what the reader of the sizes makes of it.
            let shown = h264::picture_size(&set).unwrap();
            assert!(
                shown.0 <= limits.0 && shown.1 <= limits.1,
                "{what}: shown {shown:?}"
            );

            let mut rig = rig(limits.0, limits.1);
            let outcome = rig
                .decoder
                .decode(&header(0, key()), with_first(&rest, &set));
            assert_eq!(outcome.is_ok(), taken, "{what}");
            assert_eq!(rig.decoded().len(), usize::from(taken), "{what}");
        }
    }

    #[test]
    fn every_sequence_parameter_set_of_a_frame_is_held_to_the_limits() {
        // A decoder takes the set that the picture names, or the last one with that number, and
        // neither need be the first: one set that fits does not make the others safe.
        let pictures = pictures();
        let rest = without(&pictures[0], &[SPS]);
        let fits = sps((4, 3), (0, 0));
        let too_wide = sps((5, 3), (0, 0));
        for (what, first, second) in [
            ("the second too large", &fits, &too_wide),
            ("the first too large", &too_wide, &fits),
        ] {
            let mut rig = rig(WIDTH, HEIGHT);
            let picture = with_first(&with_first(&rest, second), first);
            let outcome = rig.decoder.decode(&header(0, key()), picture);
            assert!(outcome.is_err(), "{what}");
            assert!(rig.decoded().is_empty(), "{what}: the platform was asked");
        }
    }

    #[test]
    fn a_delta_frame_is_held_to_the_limits_when_it_carries_a_sequence_parameter_set() {
        let pictures = pictures();
        let sps = units(&pictures[0])
            .into_iter()
            .find(|unit| kind(unit) == SPS)
            .unwrap();
        let carrying = with_first(&pictures[1], &sps);

        let mut small = rig(16, 16);
        // Without a set it has nothing to be held to.
        small
            .decoder
            .decode(&header(1, Flags::NONE), pictures[1].clone())
            .unwrap();
        assert!(
            small
                .decoder
                .decode(&header(2, Flags::NONE), carrying.clone())
                .is_err()
        );
        assert_eq!(small.decoded().len(), 1);

        let mut fitting = rig(WIDTH, HEIGHT);
        fitting
            .decoder
            .decode(&header(2, Flags::NONE), carrying)
            .unwrap();
        let decoded = fitting.decoded();
        assert_eq!(decoded[0].sets, [sps]);
        assert_eq!(decoded[0].sequence_sets, 1);
    }

    #[test]
    fn a_sequence_parameter_set_that_cannot_be_read_is_refused() {
        let pictures = pictures();
        let mut rig = rig(3840, 3840);
        for what in ["cut short", "zeros without end"] {
            let sps = match what {
                "cut short" => vec![0x67, 0x64, 0x00],
                _ => vec![0x67, 0, 0, 0, 0, 0, 0],
            };
            let broken = with_first(&without(&pictures[0], &[SPS]), &sps);
            assert!(
                rig.decoder.decode(&header(0, key()), broken).is_err(),
                "{what}"
            );
        }
        assert_eq!(rig.decoded().len(), 0);
    }

    #[test]
    fn a_frame_that_the_platform_refuses_is_a_decode_error_and_the_next_is_still_offered() {
        let pictures = pictures();
        let mut rig = rig(WIDTH, HEIGHT);
        rig.fake.fail_decode(-8);
        assert!(
            rig.decoder
                .decode(&header(0, key()), pictures[0].clone())
                .is_err()
        );
        rig.fake.fail_decode(0);
        rig.decoder
            .decode(&header(1, Flags::NONE), pictures[1].clone())
            .unwrap();
        assert_eq!(rig.decoded().len(), 2);
    }

    #[test]
    fn the_platform_is_told_which_frame_to_show_and_what_the_source_said() {
        let mut rig = rig(WIDTH, HEIGHT);
        rig.decoder.show(41);
        let reasons = [
            (Reason::Malformed, 1),
            (Reason::ControlOff, 2),
            (Reason::Paused, 3),
            (Reason::Locked, 4),
            (Reason::NotOnScreen, 5),
            (Reason::TooFast, 6),
            (Reason::FrameUnknown, 7),
            (Reason::PictureOutOfDate, 8),
            (Reason::WindowResized, 9),
            (Reason::OutsideWindow, 10),
            (Reason::Covered, 11),
            (Reason::NoKeyboard, 12),
            (Reason::AppWindowInFront, 13),
            (Reason::TooManyHeld, 14),
            (Reason::NotPressed, 15),
            (Reason::Other, 0),
        ];
        let mut said = vec![
            (
                SourceMessage::Config(Config {
                    epoch: 2,
                    codec: Codec::H264,
                    width: 1440,
                    height: 900,
                    fps: 30,
                }),
                (SOURCE_CONFIG, 2, CODEC_H264, 1440, 900, 30, 0, 0),
            ),
            (
                SourceMessage::Config(Config {
                    epoch: 0,
                    codec: Codec::Other,
                    width: 1,
                    height: 2,
                    fps: u16::MAX,
                }),
                (SOURCE_CONFIG, 0, 0, 1, 2, 65_535, 0, 0),
            ),
            (SourceMessage::Resume, (SOURCE_RESUME, 0, 0, 0, 0, 0, 0, 0)),
        ];
        for (reason, number) in [
            (PauseReason::Locked, 1),
            (PauseReason::Hidden, 2),
            (PauseReason::Other, 0),
        ] {
            said.push((
                SourceMessage::Pause(Pause { reason }),
                (SOURCE_PAUSE, 0, 0, 0, 0, 0, number, 0),
            ));
        }
        for (reason, number) in reasons {
            said.push((
                SourceMessage::Notice(Notice { reason, count: 7 }),
                (SOURCE_NOTICE, 0, 0, 0, 0, 0, number, 7),
            ));
        }
        let mut expected = vec![Call::Show(41)];
        for (message, (kind, epoch, codec, width, height, fps, reason, count)) in said {
            rig.decoder.source(message);
            expected.push(Call::Source(BatonSourceMessage {
                size: size_of_u32::<BatonSourceMessage>(),
                kind,
                epoch,
                codec,
                width,
                height,
                fps,
                reason,
                count,
                reserved: 0,
            }));
        }
        assert_eq!(rig.fake.calls(1), expected);
        assert!(
            rig.fake.complaints().is_empty(),
            "{:?}",
            rig.fake.complaints()
        );
    }

    #[test]
    fn what_the_source_said_about_sound_or_an_unlock_is_not_a_source_call_of_the_platform() {
        let mut rig = rig(WIDTH, HEIGHT);
        rig.decoder.source(SourceMessage::Sound(Sound {
            codec: SoundCodec::Aac,
            rate: 48_000,
            channels: 2,
            samples: 1024,
        }));
        rig.decoder.source(SourceMessage::Silent(Silent {
            reason: SilentReason::NoPermission,
        }));
        rig.decoder.source(SourceMessage::UnlockSealed(Sealed {
            challenge: Challenge::from_bytes([1; Challenge::LEN]),
            sealed: SealedKey::new(vec![3; 113]),
        }));
        rig.decoder.source(SourceMessage::UnlockDone(Done {
            outcome: Outcome::Unlocked,
        }));
        rig.decoder.source(SourceMessage::Resume);
        assert_eq!(
            rig.fake.calls(1),
            vec![Call::Source(BatonSourceMessage {
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
            })]
        );
    }

    #[test]
    fn how_the_decoder_is_doing_is_what_the_platform_says() {
        let rig = rig(WIDTH, HEIGHT);
        for (platform, expected) in [
            (0, Thermal::Nominal),
            (1, Thermal::Fair),
            (2, Thermal::Serious),
            (3, Thermal::Critical),
            (4, Thermal::Other),
            (5, Thermal::Other),
            (u32::MAX, Thermal::Other),
        ] {
            rig.fake.set_state(1234, platform);
            let state = rig.decoder.state();
            assert_eq!(state.decode_us, 1234, "thermal {platform}");
            assert_eq!(state.thermal, expected, "thermal {platform}");
        }
    }

    #[test]
    fn after_the_end_the_platform_is_called_for_nothing_but_the_release() {
        let pictures = pictures();
        let mut rig = rig(WIDTH, HEIGHT);
        rig.decoder
            .decode(&header(0, key()), pictures[0].clone())
            .unwrap();
        rig.decoder.ended();
        // The session does not call after the end, and if it did the platform would not hear.
        rig.decoder.ended();
        rig.decoder.show(0);
        rig.decoder.source(SourceMessage::Resume);
        assert!(
            rig.decoder
                .decode(&header(1, Flags::NONE), pictures[1].clone())
                .is_err()
        );
        assert_eq!(rig.decoder.state().decode_us, 0);
        let Rig { decoder, fake, .. } = rig;
        drop(decoder);
        let calls = fake.calls(1);
        assert_eq!(calls.len(), 3, "{calls:?}");
        assert!(matches!(calls[0], Call::Decode(_)));
        assert_eq!(calls[1..], [Call::Ended, Call::Release]);
        assert!(fake.complaints().is_empty(), "{:?}", fake.complaints());
    }

    #[test]
    fn a_screen_is_released_without_an_end_too_and_its_link_is_closed_first() {
        let rig = rig(WIDTH, HEIGHT);
        let open_at_release = Arc::new(Mutex::new(Vec::new()));
        let (seen, links) = (Arc::clone(&open_at_release), Arc::clone(&rig.links));
        rig.fake.on_call(move |call| {
            if *call == Call::Release {
                seen.lock().unwrap().push(links.len());
            }
        });
        assert_eq!(rig.links.len(), 1);
        let Rig {
            decoder,
            fake,
            links,
        } = rig;
        drop(decoder);
        assert_eq!(links.len(), 0);
        assert_eq!(*open_at_release.lock().unwrap(), [0]);
        assert_eq!(fake.count(|call| *call == Call::Release), 1);
        assert_eq!(fake.live_screens(), 0);
    }

    /// The form of sound that the Mac makes.
    fn aac() -> Sound {
        Sound {
            codec: SoundCodec::Aac,
            rate: 48_000,
            channels: 2,
            samples: 1024,
        }
    }

    fn played(number: u64, first: bool, lost_before: u64) -> Play {
        Play {
            number,
            capture_time: 20_000 * number,
            first,
            lost_before,
        }
    }

    /// The calls of the platform that are about sound.
    fn sound_calls(fake: &Fake) -> Vec<Call> {
        fake.calls(1)
            .into_iter()
            .filter(|call| matches!(call, Call::Sound(_) | Call::Play(_) | Call::Stopped(_)))
            .collect()
    }

    #[test]
    fn the_speaker_passes_the_form_the_packets_and_the_end_of_a_run_to_the_platform() {
        let rig = rig(WIDTH, HEIGHT);
        let mut speaker = rig.speaker();
        speaker.sound(aac());
        speaker.play(played(7, true, 0), vec![1, 2, 3]);
        speaker.play(played(10, false, 2), vec![4; 300]);
        speaker.stopped(None);
        assert_eq!(
            rig.fake.calls(1),
            [
                Call::Sound(BatonSound {
                    size: size_of_u32::<BatonSound>(),
                    codec: SOUND_CODEC_AAC,
                    rate: 48_000,
                    channels: 2,
                    samples: 1024,
                    reserved: 0,
                }),
                Call::Play(Played {
                    flags: PLAY_FIRST,
                    number: 7,
                    capture_time: 140_000,
                    lost_before: 0,
                    packet: vec![1, 2, 3],
                }),
                Call::Play(Played {
                    flags: 0,
                    number: 10,
                    capture_time: 200_000,
                    lost_before: 2,
                    packet: vec![4; 300],
                }),
                Call::Stopped(SILENT_OFF),
            ]
        );
        assert!(
            rig.fake.complaints().is_empty(),
            "{:?}",
            rig.fake.complaints()
        );
    }

    #[test]
    fn a_packet_that_the_source_made_empty_is_still_a_packet_for_the_platform() {
        // What a source sends is not checked here: a packet that the decoder cannot use is lost like
        // one that did not arrive, and the platform covers the gap.
        let rig = rig(WIDTH, HEIGHT);
        let mut speaker = rig.speaker();
        speaker.sound(aac());
        speaker.play(played(1, true, 0), Vec::new());
        let calls = sound_calls(&rig.fake);
        assert!(
            matches!(&calls[1], Call::Play(Played { number: 1, packet, .. }) if packet.is_empty()),
            "{calls:?}"
        );
        assert!(
            rig.fake.complaints().is_empty(),
            "{:?}",
            rig.fake.complaints()
        );
    }

    #[test]
    fn a_run_that_stops_tells_the_platform_why() {
        let rig = rig(WIDTH, HEIGHT);
        let mut speaker = rig.speaker();
        for why in [
            None,
            Some(SilentReason::Window),
            Some(SilentReason::NoPermission),
            Some(SilentReason::Failed),
            Some(SilentReason::Other),
        ] {
            speaker.stopped(why);
        }
        assert_eq!(
            sound_calls(&rig.fake),
            [
                Call::Stopped(SILENT_OFF),
                Call::Stopped(SILENT_WINDOW),
                Call::Stopped(SILENT_NO_PERMISSION),
                Call::Stopped(SILENT_FAILED),
                Call::Stopped(SILENT_OTHER),
            ]
        );
    }

    #[test]
    fn the_end_is_told_to_the_platform_once_whichever_of_the_two_hears_it_first() {
        for speaker_first in [false, true] {
            let mut rig = rig(WIDTH, HEIGHT);
            let mut speaker = rig.speaker();
            speaker.sound(aac());
            if speaker_first {
                speaker.ended();
                rig.decoder.ended();
            } else {
                rig.decoder.ended();
                speaker.ended();
            }
            // The session does not call after the end, and if it did the platform would not hear.
            speaker.sound(aac());
            speaker.play(played(1, true, 0), vec![1]);
            speaker.stopped(None);
            speaker.ended();
            rig.decoder.ended();
            let Rig { decoder, fake, .. } = rig;
            drop(decoder);
            drop(speaker);
            let calls = fake.calls(1);
            assert_eq!(calls.len(), 3, "speaker first: {speaker_first}: {calls:?}");
            assert!(matches!(calls[0], Call::Sound(_)));
            assert_eq!(calls[1..], [Call::Ended, Call::Release]);
            assert!(fake.complaints().is_empty(), "{:?}", fake.complaints());
        }
    }

    #[test]
    fn the_screen_is_released_when_the_last_of_the_decoder_and_the_speaker_is_dropped() {
        for decoder_first in [true, false] {
            let rig = rig(WIDTH, HEIGHT);
            let speaker = rig.speaker();
            let Rig {
                decoder,
                fake,
                links,
            } = rig;
            let released = |fake: &Fake| fake.count(|call| *call == Call::Release);
            if decoder_first {
                // The link goes with the decoder, before anything is released.
                drop(decoder);
                assert_eq!(links.len(), 0);
                assert_eq!(released(&fake), 0);
                drop(speaker);
            } else {
                drop(speaker);
                assert_eq!((links.len(), released(&fake)), (1, 0));
                drop(decoder);
                assert_eq!(links.len(), 0);
            }
            assert_eq!(released(&fake), 1, "decoder first: {decoder_first}");
            assert_eq!(fake.live_screens(), 0);
            assert!(fake.complaints().is_empty(), "{:?}", fake.complaints());
        }
    }
}
