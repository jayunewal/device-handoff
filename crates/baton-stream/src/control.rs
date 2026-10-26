// SPDX-License-Identifier: Apache-2.0

//! The messages on the control streams of a session.
//!
//! Once the viewer has accepted, each side opens one unidirectional stream of kind
//! [`KIND_CONTROL`](crate::KIND_CONTROL) to the other and keeps it open. A control stream is the
//! 17 bytes that start every stream of a session ([`header::encode_prefix`](crate::header)) and
//! then messages, one after the other, each a length and one CBOR item in the framing of
//! [`baton_proto::frame`]. The source writes [`SourceMessage`]s and the viewer writes
//! [`ViewerMessage`]s, and a message is at most [`MAX_CONTROL_MESSAGE_BYTES`] long.
//!
//! These are not requests. Nothing answers them, and they do not count against the limits on
//! requests. What they carry is what the two sides need to know about each other while frames
//! travel on streams of their own:
//!
//! - from the source, the picture it is sending ([`Config`]), a pause and the end of it
//!   ([`SourceMessage::Pause`] and [`SourceMessage::Resume`]), the reason an event from the
//!   viewer was dropped ([`gate::Notice`]) and, to a viewer that plays sound, the sound it sends
//!   ([`Sound`]) or why it sends none ([`Silent`]);
//! - from the viewer, how it is doing ([`Report`]), what it can take now ([`Want`]), a request
//!   for a key frame ([`KeyFrame`]), whether it wants to hear the source ([`Hear`]) and, when
//!   the person at the source granted control, input ([`InputBatch`]).
//!
//! Everything here is checked when it is encoded and when it is decoded, because the other
//! device is not trusted: a number out of range or a list that is too long is a violation.
//! Which messages may come when, and how often, is a rule of the session and not of the
//! messages, and lives with the session.

use baton_proto::{
    limits::{
        LimitError, MAX_CONTROL_MESSAGE_BYTES, MAX_FRAME_RATE, MAX_PICTURE_EDGE,
        MAX_SOUND_CHANNELS, MAX_SOUND_PACKET_SAMPLES, MAX_SOUND_RATE, MIN_SOUND_PACKET_SAMPLES,
        MIN_SOUND_RATE,
    },
    message::Message,
    stream::Codec,
};
use serde::{Deserialize, Serialize};

use crate::{
    gate,
    input::{self, Event, InputError},
};

/// The picture that the source sends from now on.
///
/// The source sends one before the first frame of a session and another for each change of
/// the size of the picture. Frames and control messages travel on streams of their own, so a
/// frame of a new epoch can arrive before the message that describes it; a viewer decodes it all
/// the same, because the parameter sets are in the key frame, and uses this for the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// The epoch of the frames that this describes ([`FrameHeader::epoch`](crate::header)).
    pub epoch: u32,
    /// How the pictures are coded.
    pub codec: Codec,
    /// The width of the picture in pixels. The picture shows the whole window, scaled to this size.
    pub width: u32,
    /// The height of the picture in pixels.
    pub height: u32,
    /// The most frames a second that the source sends.
    pub fps: u16,
}

/// Why the source stopped sending pictures for a while.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PauseReason {
    /// The Mac is locked, or its display is off. Nothing documented can capture it, so nothing
    /// is shown or typed until the person unlocks.
    Locked,
    /// The shared window is minimised or not on screen. Capture of a single window pauses with it.
    Hidden,
    /// Any reason this version does not know about.
    #[serde(other)]
    Other,
}

/// The source has stopped sending pictures. Input is dropped until [`SourceMessage::Resume`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pause {
    /// Why, so that the viewer can say it.
    pub reason: PauseReason,
}

/// How the sound is coded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SoundCodec {
    /// AAC-LC. A packet is one access unit, as the encoder makes it, with no ADTS header and no
    /// other framing. The viewer describes the stream to its decoder from the rate, the channels
    /// and the samples of [`Sound`].
    Aac,
    /// Any codec this version does not know about.
    #[serde(other)]
    Other,
}

/// The sound that the source sends from now on, in packets on streams of kind
/// [`KIND_SOUND`](crate::KIND_SOUND).
///
/// The source sends one before the first packet of a run of sound. Packets and control messages
/// travel on streams of their own, so a packet can arrive before the message that describes it;
/// a viewer that has not read the message yet drops the packet as if it had been lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sound {
    /// How the packets are coded.
    pub codec: SoundCodec,
    /// The sampling rate in hertz, from [`MIN_SOUND_RATE`] to [`MAX_SOUND_RATE`].
    pub rate: u32,
    /// How many channels there are: 1 or 2.
    pub channels: u8,
    /// How many samples of each channel one packet holds, from [`MIN_SOUND_PACKET_SAMPLES`] to
    /// [`MAX_SOUND_PACKET_SAMPLES`]. A packet lasts this many samples at the rate: 1024 at 48000
    /// hertz is 21.3 milliseconds.
    pub samples: u16,
}

/// Why the source sends no sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SilentReason {
    /// The session is of a window. Sound goes only with a whole screen.
    Window,
    /// The operating system of the source has not allowed its program to record the sound of the
    /// source, and the person at the source has to allow it.
    NoPermission,
    /// Something went wrong at the source that the viewer did not cause.
    Failed,
    /// Any reason this version does not know about.
    #[serde(other)]
    Other,
}

/// The source sends no sound, and says why.
///
/// It answers a [`Hear`] that it cannot carry out, and it ends a run of sound that was going
/// when the cause comes up during the run, for example when the person at the source takes the
/// permission away. No packet follows it until the next [`Sound`]. The source may begin that run
/// by itself when the cause is gone and the last [`Hear`] was `on`, and the viewer can ask it to
/// try again with another [`Hear`] that is `on`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Silent {
    /// Why, so that the viewer can say it.
    pub reason: SilentReason,
}

/// A message from the source to the viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceMessage {
    /// The picture that follows.
    Config(Config),
    /// Pictures stop for a while.
    Pause(Pause),
    /// Pictures go on.
    Resume,
    /// Events from the viewer were dropped, and why. At most one in a second.
    Notice(gate::Notice),
    /// The sound that follows. Only to a viewer that advertised
    /// [`capability::STREAM_HEAR`](baton_proto::message::capability::STREAM_HEAR).
    Sound(Sound),
    /// There is no sound, and why. Only to a viewer that advertised
    /// [`capability::STREAM_HEAR`](baton_proto::message::capability::STREAM_HEAR).
    Silent(Silent),
}

/// How hot the viewer's device is.
///
/// The four states are those that iOS reports. A source that is told `Serious` or `Critical`
/// lowers the frame rate or the size of the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Thermal {
    /// No limit.
    Nominal,
    /// Slightly warm.
    Fair,
    /// Hot: the system is limiting what the device can do.
    Serious,
    /// As hot as it gets: the system is about to limit the device further.
    Critical,
    /// Any state this version does not know about.
    #[serde(other)]
    Other,
}

/// How the viewer is doing.
///
/// The viewer sends one a few times a second while pictures come, and at least every two
/// seconds. A source that gets none for ten seconds stops encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// The newest frame that was decoded and shown. Left out before the first picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown: Option<u64>,
    /// How long the viewer takes to decode a frame, in microseconds, as it smooths it.
    pub decode_us: u32,
    /// How many frames the viewer has dropped in this session, counted from the start so that a
    /// report that is not read changes nothing. They are frames it received and did not decode.
    pub dropped: u64,
    /// The thermal state of the device.
    pub thermal: Thermal,
}

/// The largest picture and the highest frame rate that the viewer can take now.
///
/// They change when the phone is turned. The source never sends more than the viewer said when it
/// accepted, whatever this says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Want {
    /// The widest picture in pixels.
    pub width: u32,
    /// The highest picture in pixels.
    pub height: u32,
    /// The most frames a second.
    pub fps: u16,
}

/// The viewer cannot decode and asks for a key frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyFrame {
    /// The number of the newest frame that the viewer has handed to its decoder. A key frame
    /// numbered above it answers the request, and one at or below it does not.
    pub tried: u64,
}

/// What the person did on the viewer since the last batch. Only to a source that granted control.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputBatch {
    /// The events in the order in which they happened, at least one and at most
    /// [`input::MAX_BATCH`].
    pub events: Vec<Event>,
}

/// The viewer wants to hear the source, or does not any more.
///
/// A viewer that has not said it wants to hear gets no sound. An `on` asks the source to begin a
/// run of sound if none is going, and to say why with [`Silent`] if it cannot; while a run goes
/// it changes nothing. An `off` ends the run, if there is one, and the source says nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hear {
    /// Whether to send sound.
    pub on: bool,
}

/// A message from the viewer to the source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ViewerMessage {
    /// How the viewer is doing.
    Report(Report),
    /// What it can take now.
    Want(Want),
    /// It needs a key frame.
    KeyFrame(KeyFrame),
    /// What the person did.
    Input(InputBatch),
    /// It wants to hear the source, or it does not. Only to a source that advertised
    /// [`capability::STREAM_SOUND`](baton_proto::message::capability::STREAM_SOUND).
    Hear(Hear),
}

impl Config {
    fn validate(&self) -> Result<(), LimitError> {
        check_edge("picture width", self.width)?;
        check_edge("picture height", self.height)?;
        check_rate(self.fps)
    }
}

impl Want {
    fn validate(&self) -> Result<(), LimitError> {
        check_edge("wanted picture width", self.width)?;
        check_edge("wanted picture height", self.height)?;
        check_rate(self.fps)
    }
}

impl Sound {
    fn validate(self) -> Result<(), LimitError> {
        check_between("sound rate", self.rate, MIN_SOUND_RATE, MAX_SOUND_RATE)?;
        check_between(
            "sound channels",
            u32::from(self.channels),
            1,
            u32::from(MAX_SOUND_CHANNELS),
        )?;
        check_between(
            "samples in a packet of sound",
            u32::from(self.samples),
            u32::from(MIN_SOUND_PACKET_SAMPLES),
            u32::from(MAX_SOUND_PACKET_SAMPLES),
        )
    }
}

impl InputBatch {
    fn validate(&self) -> Result<(), LimitError> {
        if self.events.is_empty() {
            return Err(LimitError::Empty {
                field: "input events",
            });
        }
        if self.events.len() > input::MAX_BATCH {
            return Err(LimitError::TooMany {
                field: "input events",
                count: self.events.len(),
                max: input::MAX_BATCH,
            });
        }
        self.events.iter().try_for_each(|event| {
            event.validate().map_err(|error| LimitError::Invalid {
                field: "input event",
                reason: input_problem(error),
            })
        })
    }
}

fn check_edge(field: &'static str, value: u32) -> Result<(), LimitError> {
    if (1..=MAX_PICTURE_EDGE).contains(&value) {
        Ok(())
    } else {
        Err(LimitError::Invalid {
            field,
            reason: "is not between 1 and the most there may be",
        })
    }
}

fn check_rate(fps: u16) -> Result<(), LimitError> {
    if (1..=MAX_FRAME_RATE).contains(&fps) {
        Ok(())
    } else {
        Err(LimitError::Invalid {
            field: "frame rate",
            reason: "is not between 1 and the most there may be",
        })
    }
}

fn check_between(field: &'static str, value: u32, least: u32, most: u32) -> Result<(), LimitError> {
    if (least..=most).contains(&value) {
        Ok(())
    } else {
        Err(LimitError::Invalid {
            field,
            reason: "is not between the least and the most there may be",
        })
    }
}

/// What is wrong with an event, in words that are the same for every event, so that a log says
/// what the problem was and nothing of what the person did.
const fn input_problem(error: InputError) -> &'static str {
    match error {
        InputError::NotFinite => "a number is not finite",
        InputError::OutOfRange => "a position is outside 0 to 1",
        InputError::ScrollTooLarge => "a scroll step is too large",
        InputError::TextTooLong => "the text of a key is too long",
        InputError::TextHasControl => "the text of a key has a control character",
        InputError::KeyCode => "the code of a key is not one of the keyboard page",
        InputError::TextOnly => "an event with no key does more than type its text once",
        InputError::Modifiers => "the modifiers have bits that are not defined",
        InputError::TooManyEvents => "a batch has too many events",
    }
}

impl Message for SourceMessage {
    const MAX_FRAME_LEN: usize = MAX_CONTROL_MESSAGE_BYTES;

    fn validate(&self) -> Result<(), LimitError> {
        match self {
            Self::Config(config) => config.validate(),
            Self::Sound(sound) => sound.validate(),
            Self::Notice(notice) if notice.count == 0 => Err(LimitError::Invalid {
                field: "number of dropped events",
                reason: "is 0",
            }),
            Self::Pause(_) | Self::Resume | Self::Notice(_) | Self::Silent(_) => Ok(()),
        }
    }
}

impl Message for ViewerMessage {
    const MAX_FRAME_LEN: usize = MAX_CONTROL_MESSAGE_BYTES;

    fn validate(&self) -> Result<(), LimitError> {
        match self {
            Self::Report(_) | Self::KeyFrame(_) | Self::Hear(_) => Ok(()),
            Self::Want(want) => want.validate(),
            Self::Input(batch) => batch.validate(),
        }
    }
}

#[cfg(test)]
mod tests {
    use baton_proto::frame::{self, FrameError};
    use ciborium::{Value, cbor};
    use proptest::prelude::*;

    use super::*;
    use crate::{
        gate::{Notice, Reason},
        input::{Button, Modifiers, PointerAction},
    };

    const REASONS: [Reason; 16] = [
        Reason::Malformed,
        Reason::ControlOff,
        Reason::Paused,
        Reason::Locked,
        Reason::NotOnScreen,
        Reason::TooFast,
        Reason::FrameUnknown,
        Reason::PictureOutOfDate,
        Reason::WindowResized,
        Reason::OutsideWindow,
        Reason::Covered,
        Reason::NoKeyboard,
        Reason::AppWindowInFront,
        Reason::TooManyHeld,
        Reason::NotPressed,
        Reason::Other,
    ];

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write;
        bytes.iter().fold(String::new(), |mut out, b| {
            write!(out, "{b:02x}").unwrap();
            out
        })
    }

    fn cbor_bytes(value: &Value) -> Vec<u8> {
        let mut out = Vec::new();
        ciborium::into_writer(value, &mut out).unwrap();
        out
    }

    fn config() -> Config {
        Config {
            epoch: 1,
            codec: Codec::H264,
            width: 1440,
            height: 900,
            fps: 60,
        }
    }

    /// What the first version sends: AAC-LC at 48000 hertz in two channels, 1024 samples a packet.
    fn sound() -> Sound {
        Sound {
            codec: SoundCodec::Aac,
            rate: 48_000,
            channels: 2,
            samples: 1024,
        }
    }

    fn press(frame: u64) -> Event {
        Event::Pointer {
            frame,
            x: 0.5,
            y: 0.25,
            modifiers: Modifiers::NONE,
            action: PointerAction::Down(Button::Left),
        }
    }

    /// The key `a` going down, with the text that the system attached to it, if any.
    fn key_a(text: Option<&str>) -> Event {
        Event::Key {
            code: 4,
            down: true,
            modifiers: Modifiers::NONE,
            repeat: false,
            text: text.map(str::to_owned),
        }
    }

    fn typed(text: &str) -> Event {
        Event::Key {
            code: 0,
            down: true,
            modifiers: Modifiers::NONE,
            repeat: false,
            text: Some(text.to_owned()),
        }
    }

    fn batch(events: Vec<Event>) -> ViewerMessage {
        ViewerMessage::Input(InputBatch { events })
    }

    fn encoded<M: Message>(message: &M) -> String {
        hex(&frame::encode(message).unwrap())
    }

    #[test]
    fn the_messages_of_the_source_have_known_wire_encodings() {
        // The exact bytes are the protocol (docs/protocol.md section 7). If this fails, the wire
        // format changed.
        assert_eq!(
            encoded(&SourceMessage::Config(config())),
            // {"Config": {"epoch": 1, "codec": "H264", "width": 1440, "height": 900, "fps": 60}}
            "00000034a166436f6e666967a56565706f63680165636f646563644832363465776964746819\
             05a06668656967687419038463667073183c"
        );
        assert_eq!(
            encoded(&SourceMessage::Pause(Pause {
                reason: PauseReason::Locked
            })),
            // {"Pause": {"reason": "Locked"}}
            "00000016a1655061757365a166726561736f6e664c6f636b6564"
        );
        assert_eq!(
            encoded(&SourceMessage::Resume),
            // "Resume"
            "0000000766526573756d65"
        );
        assert_eq!(
            encoded(&SourceMessage::Notice(Notice {
                reason: Reason::Covered,
                count: 3
            })),
            // {"Notice": {"reason": "Covered", "count": 3}}
            "0000001fa1664e6f74696365a266726561736f6e67436f766572656465636f756e7403"
        );
        assert_eq!(
            encoded(&SourceMessage::Sound(sound())),
            concat!(
                "0000002f",           // the frame: 47 bytes follow
                "a165536f756e64",     // {"Sound":
                "a4",                 //   map of 4
                "65636f646563",       //   "codec":
                "63416163",           //     "Aac",
                "6472617465",         //   "rate":
                "19bb80",             //     48000,
                "686368616e6e656c73", //   "channels":
                "02",                 //     2,
                "6773616d706c6573",   //   "samples":
                "190400",             //     1024
            )
        );
        assert_eq!(
            encoded(&SourceMessage::Silent(Silent {
                reason: SilentReason::NoPermission
            })),
            // {"Silent": {"reason": "NoPermission"}}
            "0000001da16653696c656e74a166726561736f6e6c4e6f5065726d697373696f6e"
        );
    }

    #[test]
    fn the_messages_of_the_viewer_have_known_wire_encodings() {
        let report = |shown| {
            ViewerMessage::Report(Report {
                shown,
                decode_us: 2500,
                dropped: 3,
                thermal: Thermal::Fair,
            })
        };
        assert_eq!(
            encoded(&report(Some(41))),
            // {"Report": {"shown": 41, "decode_us": 2500, "dropped": 3, "thermal": "Fair"}}
            "00000034a1665265706f7274a46573686f776e1829696465636f64655f75731909c4\
             6764726f707065640367746865726d616c6446616972"
        );
        // Before the first picture there is nothing to name, and the key is left out.
        assert_eq!(
            encoded(&report(None)),
            // {"Report": {"decode_us": 2500, "dropped": 3, "thermal": "Fair"}}
            "0000002ca1665265706f7274a3696465636f64655f75731909c46764726f7070656403\
             67746865726d616c6446616972"
        );
        assert_eq!(
            encoded(&ViewerMessage::Want(Want {
                width: 2796,
                height: 1290,
                fps: 120
            })),
            // {"Want": {"width": 2796, "height": 1290, "fps": 120}}
            "00000020a16457616e74a3657769647468190aec6668656967687419050a636670731878"
        );
        assert_eq!(
            encoded(&ViewerMessage::KeyFrame(KeyFrame { tried: 5 })),
            // {"KeyFrame": {"tried": 5}}
            "00000012a1684b65794672616d65a165747269656405"
        );
        assert_eq!(
            encoded(&ViewerMessage::Hear(Hear { on: true })),
            // {"Hear": {"on": true}}
            "0000000ba16448656172a1626f6ef5"
        );
        assert_eq!(
            encoded(&ViewerMessage::Hear(Hear { on: false })),
            // {"Hear": {"on": false}}
            "0000000ba16448656172a1626f6ef4"
        );
        assert_eq!(
            encoded(&batch(vec![press(7)])),
            // {"Input": {"events": [{"Pointer": {"frame": 7, "x": 0.5, "y": 0.25,
            //                                     "modifiers": 0, "action": {"Down": "Left"}}}]}}
            "00000048a165496e707574a1666576656e747381a167506f696e746572a5656672616d65076178\
             f938006179f93400696d6f646966696572730066616374696f6ea164446f776e644c656674"
        );
        // A key with no text leaves `text` out, like the other optional fields, and does not
        // write a null.
        assert_eq!(
            encoded(&batch(vec![key_a(None)])),
            // {"Input": {"events": [{"Key": {"code": 4, "down": true, "modifiers": 0,
            //                                "repeat": false}}]}}
            "00000035a165496e707574a1666576656e747381a1634b6579a464636f64650464646f776ef5\
             696d6f646966696572730066726570656174f4"
        );
        assert_eq!(
            encoded(&batch(vec![key_a(Some("a"))])),
            // The same with "text": "a" after "repeat".
            "0000003ca165496e707574a1666576656e747381a1634b6579a564636f64650464646f776ef5\
             696d6f646966696572730066726570656174f464746578746161"
        );
    }

    #[test]
    fn a_unit_variant_may_also_be_a_map_with_null() {
        // Section 2 of the protocol: some encoders write `Resume` this way, and it is accepted.
        let payload = cbor_bytes(&cbor!({ "Resume" => null }).unwrap());
        assert_eq!(
            frame::decode_payload::<SourceMessage>(&payload),
            Ok(SourceMessage::Resume)
        );
    }

    #[test]
    fn a_number_with_a_fraction_is_a_float_of_any_width_and_never_an_integer() {
        // Section 2 of the protocol. Each case writes one number of a known payload in another
        // way: as a half, a single or a double float, or as an integer.
        let payload_of =
            |message: &ViewerMessage| frame::encode(message).unwrap()[frame::HEADER_LEN..].to_vec();
        let with = |payload: &[u8], old: &[u8], new: &[u8]| {
            let mut found = payload
                .windows(old.len())
                .enumerate()
                .filter(|(_, window)| *window == old);
            let (at, _) = found.next().expect("the number is in the payload");
            assert!(found.next().is_none(), "the number is in the payload twice");
            [&payload[..at], new, &payload[at + old.len()..]].concat()
        };
        let read = |payload: &[u8]| frame::decode_payload::<ViewerMessage>(payload);

        // The `x` of a press is 0.5, which is the half float f93800 on the wire.
        let payload = payload_of(&batch(vec![press(7)]));
        let half = [0xf9, 0x38, 0x00];
        for x in [
            &half[..],
            &[0xfa, 0x3f, 0x00, 0x00, 0x00],
            &[0xfb, 0x3f, 0xe0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ] {
            assert_eq!(
                read(&with(&payload, &half, x)),
                Ok(batch(vec![press(7)])),
                "{}",
                hex(x)
            );
        }
        // 0 and 1 are the edges of the picture, and a position that is written as an integer is
        // not read.
        for x in [0x00, 0x01] {
            assert!(
                matches!(
                    read(&with(&payload, &half, &[x])),
                    Err(FrameError::Decode(_))
                ),
                "the integer {x} was taken for a position"
            );
        }

        // A scroll step is the same: the `dx` of a scroll is 0.0, the half float f90000.
        let scroll = || {
            batch(vec![Event::Pointer {
                frame: 7,
                x: 0.5,
                y: 0.25,
                modifiers: Modifiers::NONE,
                action: PointerAction::Scroll {
                    dx: 0.0,
                    dy: -120.0,
                },
            }])
        };
        let payload = payload_of(&scroll());
        let zero = [0xf9, 0x00, 0x00];
        for dx in [
            &zero[..],
            &[0xfa, 0x00, 0x00, 0x00, 0x00],
            &[0xfb, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ] {
            assert_eq!(
                read(&with(&payload, &zero, dx)),
                Ok(scroll()),
                "{}",
                hex(dx)
            );
        }
        assert!(matches!(
            read(&with(&payload, &zero, &[0x00])),
            Err(FrameError::Decode(_))
        ));
    }

    #[test]
    fn what_a_later_version_may_add_to_an_enum_reads_as_other() {
        let decode = |value: Value| frame::decode_payload::<SourceMessage>(&cbor_bytes(&value));
        assert_eq!(
            decode(cbor!({ "Pause" => { "reason" => "Asleep" } }).unwrap()),
            Ok(SourceMessage::Pause(Pause {
                reason: PauseReason::Other
            }))
        );
        assert_eq!(
            decode(cbor!({ "Notice" => { "reason" => "Glitter", "count" => 2 } }).unwrap()),
            Ok(SourceMessage::Notice(Notice {
                reason: Reason::Other,
                count: 2
            }))
        );
        // A codec that the viewer does not know is read, and the viewer ends the session.
        assert_eq!(
            decode(
                cbor!({ "Config" => { "epoch" => 0, "codec" => "AV1", "width" => 640,
                                       "height" => 480, "fps" => 30 } })
                .unwrap()
            ),
            Ok(SourceMessage::Config(Config {
                epoch: 0,
                codec: Codec::Other,
                width: 640,
                height: 480,
                fps: 30
            }))
        );
        // The same for sound: a reason and a codec of a later version are read, as `Other`.
        assert_eq!(
            decode(cbor!({ "Silent" => { "reason" => "Muted" } }).unwrap()),
            Ok(SourceMessage::Silent(Silent {
                reason: SilentReason::Other
            }))
        );
        assert_eq!(
            decode(
                cbor!({ "Sound" => { "codec" => "Opus", "rate" => 48000, "channels" => 2,
                                      "samples" => 960 } })
                .unwrap()
            ),
            Ok(SourceMessage::Sound(Sound {
                codec: SoundCodec::Other,
                rate: 48_000,
                channels: 2,
                samples: 960
            }))
        );
        let report =
            cbor!({ "Report" => { "decode_us" => 1, "dropped" => 0, "thermal" => "Scorching" } })
                .unwrap();
        assert_eq!(
            frame::decode_payload::<ViewerMessage>(&cbor_bytes(&report)),
            Ok(ViewerMessage::Report(Report {
                shown: None,
                decode_us: 1,
                dropped: 0,
                thermal: Thermal::Other
            }))
        );
    }

    #[test]
    fn a_message_that_this_version_does_not_know_is_not_skipped() {
        // New kinds of message need a capability that both sides advertised (section 5), so an
        // unknown one is a violation and not something to read past.
        let unknown = cbor_bytes(&cbor!({ "Teleport" => { "to" => "Mars" } }).unwrap());
        assert!(matches!(
            frame::decode_payload::<SourceMessage>(&unknown),
            Err(FrameError::Decode(_))
        ));
        assert!(matches!(
            frame::decode_payload::<ViewerMessage>(&unknown),
            Err(FrameError::Decode(_))
        ));
        // The two directions do not read each other's messages.
        let tried = cbor_bytes(&cbor!({ "KeyFrame" => { "tried" => 5 } }).unwrap());
        assert!(frame::decode_payload::<SourceMessage>(&tried).is_err());
        let resume = cbor_bytes(&Value::Text("Resume".into()));
        assert!(frame::decode_payload::<ViewerMessage>(&resume).is_err());
        let hear = cbor_bytes(&cbor!({ "Hear" => { "on" => true } }).unwrap());
        assert!(frame::decode_payload::<SourceMessage>(&hear).is_err());
        let silent = cbor_bytes(&cbor!({ "Silent" => { "reason" => "Failed" } }).unwrap());
        assert!(frame::decode_payload::<ViewerMessage>(&silent).is_err());
        let sound = cbor_bytes(
            &cbor!({ "Sound" => { "codec" => "Aac", "rate" => 48000, "channels" => 2,
                                   "samples" => 1024 } })
            .unwrap(),
        );
        assert!(frame::decode_payload::<ViewerMessage>(&sound).is_err());
    }

    #[test]
    fn a_hear_that_does_not_say_whether_is_not_read() {
        // `on` has no default: a message that leaves it out says nothing, and is refused.
        let payload = cbor_bytes(&cbor!({ "Hear" => {} }).unwrap());
        assert!(matches!(
            frame::decode_payload::<ViewerMessage>(&payload),
            Err(FrameError::Decode(_))
        ));
        // And it is a boolean, not a number.
        let payload = cbor_bytes(&cbor!({ "Hear" => { "on" => 1 } }).unwrap());
        assert!(matches!(
            frame::decode_payload::<ViewerMessage>(&payload),
            Err(FrameError::Decode(_))
        ));
    }

    #[test]
    fn the_format_of_sound_is_checked_when_it_is_written_and_when_it_is_read() {
        let limit = |result: Result<Vec<u8>, FrameError>| {
            matches!(result, Err(FrameError::Limit(LimitError::Invalid { .. })))
        };
        let said = |rate, channels, samples| {
            frame::encode(&SourceMessage::Sound(Sound {
                rate,
                channels,
                samples,
                ..sound()
            }))
        };
        assert!(said(48_000, 2, 1024).is_ok());
        assert!(said(44_100, 1, 960).is_ok());
        // The edges are written as numbers and not as the constants, so that a change of a limit
        // is a change of this test, and of the table in section 6 of the protocol.
        assert!(said(8_000, 1, 256).is_ok());
        assert!(said(96_000, 2, 2048).is_ok());
        assert!(limit(said(7_999, 2, 1024)));
        assert!(limit(said(0, 2, 1024)));
        assert!(limit(said(96_001, 2, 1024)));
        assert!(limit(said(48_000, 0, 1024)));
        assert!(limit(said(48_000, 3, 1024)));
        assert!(limit(said(48_000, 2, 255)));
        assert!(limit(said(48_000, 2, 0)));
        assert!(limit(said(48_000, 2, 2049)));

        // A format that a later version could send is still checked on the way in.
        let payload = cbor_bytes(
            &cbor!({ "Sound" => { "codec" => "Aac", "rate" => 48000, "channels" => 6,
                                   "samples" => 1024 } })
            .unwrap(),
        );
        assert!(matches!(
            frame::decode_payload::<SourceMessage>(&payload),
            Err(FrameError::Limit(LimitError::Invalid { .. }))
        ));
        // A number that does not fit the field is not read as a smaller one.
        let payload = cbor_bytes(
            &cbor!({ "Sound" => { "codec" => "Aac", "rate" => 48000, "channels" => 258,
                                   "samples" => 1024 } })
            .unwrap(),
        );
        assert!(matches!(
            frame::decode_payload::<SourceMessage>(&payload),
            Err(FrameError::Decode(_))
        ));
    }

    #[test]
    fn a_silent_message_and_a_hear_have_no_limits_to_break() {
        for reason in [
            SilentReason::Window,
            SilentReason::NoPermission,
            SilentReason::Failed,
            SilentReason::Other,
        ] {
            assert!(frame::encode(&SourceMessage::Silent(Silent { reason })).is_ok());
        }
        for on in [false, true] {
            assert!(frame::encode(&ViewerMessage::Hear(Hear { on })).is_ok());
        }
    }

    #[test]
    fn a_field_that_a_later_version_adds_is_ignored() {
        let payload = cbor_bytes(
            &cbor!({ "KeyFrame" => { "tried" => 5, "why" => "the decoder choked" } }).unwrap(),
        );
        assert_eq!(
            frame::decode_payload::<ViewerMessage>(&payload),
            Ok(ViewerMessage::KeyFrame(KeyFrame { tried: 5 }))
        );
    }

    #[test]
    fn a_picture_and_a_rate_outside_the_limits_are_refused() {
        let limit = |result: Result<Vec<u8>, FrameError>| {
            matches!(result, Err(FrameError::Limit(LimitError::Invalid { .. })))
        };
        let config = |width, height, fps| {
            frame::encode(&SourceMessage::Config(Config {
                width,
                height,
                fps,
                ..config()
            }))
        };
        assert!(config(1, 1, 1).is_ok());
        assert!(config(MAX_PICTURE_EDGE, MAX_PICTURE_EDGE, MAX_FRAME_RATE).is_ok());
        assert!(limit(config(0, 900, 60)));
        assert!(limit(config(1440, 0, 60)));
        assert!(limit(config(MAX_PICTURE_EDGE + 1, 900, 60)));
        assert!(limit(config(1440, MAX_PICTURE_EDGE + 1, 60)));
        assert!(limit(config(1440, 900, 0)));
        assert!(limit(config(1440, 900, MAX_FRAME_RATE + 1)));

        let want =
            |width, height, fps| frame::encode(&ViewerMessage::Want(Want { width, height, fps }));
        assert!(want(2796, 1290, 120).is_ok());
        assert!(limit(want(0, 1290, 60)));
        assert!(limit(want(2796, MAX_PICTURE_EDGE + 1, 60)));
        assert!(limit(want(2796, 1290, 0)));
        assert!(limit(want(2796, 1290, MAX_FRAME_RATE + 1)));
    }

    #[test]
    fn a_limit_is_checked_when_a_message_is_read_too() {
        let payload =
            cbor_bytes(&cbor!({ "Want" => { "width" => 0, "height" => 1, "fps" => 1 } }).unwrap());
        assert!(matches!(
            frame::decode_payload::<ViewerMessage>(&payload),
            Err(FrameError::Limit(LimitError::Invalid { .. }))
        ));
    }

    #[test]
    fn a_notice_of_no_events_is_refused() {
        let notice = |count| {
            frame::encode(&SourceMessage::Notice(Notice {
                reason: Reason::Covered,
                count,
            }))
        };
        assert!(notice(1).is_ok());
        assert!(notice(u32::MAX).is_ok());
        assert!(matches!(
            notice(0),
            Err(FrameError::Limit(LimitError::Invalid { .. }))
        ));
    }

    #[test]
    fn a_batch_of_input_has_at_least_one_event_and_at_most_the_limit() {
        let many = |count| batch((0..count).map(|_| press(0)).collect());
        assert!(frame::encode(&many(1)).is_ok());
        assert!(frame::encode(&many(input::MAX_BATCH)).is_ok());
        assert!(matches!(
            frame::encode(&many(0)),
            Err(FrameError::Limit(LimitError::Empty { .. }))
        ));
        assert!(matches!(
            frame::encode(&many(input::MAX_BATCH + 1)),
            Err(FrameError::Limit(LimitError::TooMany { count, max, .. }))
                if count == input::MAX_BATCH + 1 && max == input::MAX_BATCH
        ));
    }

    #[test]
    fn an_event_that_breaks_its_limits_refuses_the_whole_batch() {
        let moved_to = |x| Event::Pointer {
            frame: 0,
            x,
            y: 0.5,
            modifiers: Modifiers::NONE,
            action: PointerAction::Move,
        };
        for bad in [
            moved_to(1.5),
            moved_to(f32::NAN),
            typed(&"x".repeat(input::MAX_KEY_TEXT_UNITS + 1)),
            typed("\u{7}"),
        ] {
            let result = frame::encode(&batch(vec![press(1), bad, press(2)]));
            assert!(
                matches!(result, Err(FrameError::Limit(LimitError::Invalid { .. }))),
                "{result:?}"
            );
        }
    }

    #[test]
    fn what_a_person_typed_is_not_in_the_error_or_in_the_log() {
        // A password is typed one key at a time. The text of a key stays out of a log, and out
        // of the message that says why a batch was refused.
        let secret = "hunter2\u{7}";
        let message = batch(vec![typed(secret)]);
        assert!(!format!("{message:?}").contains("hunter2"));
        let error = frame::encode(&message).unwrap_err();
        assert!(!format!("{error:?} {error}").contains("hunter2"), "{error}");
    }

    #[test]
    fn the_largest_message_is_far_below_the_limit() {
        // 64 keys, each with the longest text (20 units of three bytes), are the biggest thing a
        // viewer can send.
        let text = "\u{3042}".repeat(input::MAX_KEY_TEXT_UNITS);
        let events = (0..input::MAX_BATCH)
            .map(|_| Event::Key {
                code: input::MAX_KEY_CODE,
                down: true,
                modifiers: Modifiers::SHIFT | Modifiers::CONTROL | Modifiers::OPTION,
                repeat: true,
                text: Some(text.clone()),
            })
            .collect();
        let frame = frame::encode(&batch(events)).unwrap();
        assert!(
            frame.len() < MAX_CONTROL_MESSAGE_BYTES / 4,
            "{}",
            frame.len()
        );
    }

    #[test]
    fn the_limit_is_the_one_of_the_protocol() {
        assert_eq!(<SourceMessage as Message>::MAX_FRAME_LEN, 64 * 1024);
        assert_eq!(<ViewerMessage as Message>::MAX_FRAME_LEN, 64 * 1024);
        let largest = u32::try_from(MAX_CONTROL_MESSAGE_BYTES).unwrap();
        assert_eq!(
            frame::payload_len::<ViewerMessage>(largest.to_be_bytes()),
            Ok(MAX_CONTROL_MESSAGE_BYTES)
        );
        assert!(matches!(
            frame::payload_len::<ViewerMessage>((largest + 1).to_be_bytes()),
            Err(FrameError::TooLarge { .. })
        ));
    }

    fn modifiers() -> impl Strategy<Value = Modifiers> {
        (0u8..32).prop_map(|bits| Modifiers::from_bits(bits).unwrap())
    }

    fn button() -> impl Strategy<Value = Button> {
        prop_oneof![
            Just(Button::Left),
            Just(Button::Right),
            Just(Button::Middle)
        ]
    }

    fn action() -> impl Strategy<Value = PointerAction> {
        prop_oneof![
            Just(PointerAction::Move),
            button().prop_map(PointerAction::Down),
            button().prop_map(PointerAction::Up),
            (-10_000.0f32..=10_000.0, -10_000.0f32..=10_000.0)
                .prop_map(|(dx, dy)| PointerAction::Scroll { dx, dy }),
        ]
    }

    fn event() -> impl Strategy<Value = Event> {
        prop_oneof![
            (
                any::<u64>(),
                0.0f32..=1.0,
                0.0f32..=1.0,
                modifiers(),
                action()
            )
                .prop_map(|(frame, x, y, modifiers, action)| Event::Pointer {
                    frame,
                    x,
                    y,
                    modifiers,
                    action
                }),
            // A key of the keyboard page, with the text it produced or none. Up to ten characters
            // of up to two UTF-16 units each, which is the longest text there can be.
            (
                1..=input::MAX_KEY_CODE,
                any::<bool>(),
                modifiers(),
                any::<bool>(),
                proptest::option::of("\\PC{0,10}")
            )
                .prop_map(|(code, down, modifiers, repeat, text)| Event::Key {
                    code,
                    down,
                    modifiers,
                    repeat,
                    text
                }),
            // Text with no key: it goes down once, with some text.
            (modifiers(), "\\PC{1,10}").prop_map(|(modifiers, text)| Event::Key {
                code: input::NO_KEY,
                down: true,
                modifiers,
                repeat: false,
                text: Some(text)
            }),
        ]
    }

    fn thermal() -> impl Strategy<Value = Thermal> {
        prop_oneof![
            Just(Thermal::Nominal),
            Just(Thermal::Fair),
            Just(Thermal::Serious),
            Just(Thermal::Critical),
            Just(Thermal::Other),
        ]
    }

    fn source_message() -> impl Strategy<Value = SourceMessage> {
        prop_oneof![
            (
                any::<u32>(),
                prop_oneof![Just(Codec::H264), Just(Codec::Other)],
                1..=MAX_PICTURE_EDGE,
                1..=MAX_PICTURE_EDGE,
                1..=MAX_FRAME_RATE
            )
                .prop_map(|(epoch, codec, width, height, fps)| {
                    SourceMessage::Config(Config {
                        epoch,
                        codec,
                        width,
                        height,
                        fps,
                    })
                }),
            prop_oneof![
                Just(PauseReason::Locked),
                Just(PauseReason::Hidden),
                Just(PauseReason::Other)
            ]
            .prop_map(|reason| SourceMessage::Pause(Pause { reason })),
            Just(SourceMessage::Resume),
            (0..REASONS.len(), 1..=u32::MAX).prop_map(|(reason, count)| {
                SourceMessage::Notice(Notice {
                    reason: REASONS[reason],
                    count,
                })
            }),
            (
                prop_oneof![Just(SoundCodec::Aac), Just(SoundCodec::Other)],
                MIN_SOUND_RATE..=MAX_SOUND_RATE,
                1..=MAX_SOUND_CHANNELS,
                MIN_SOUND_PACKET_SAMPLES..=MAX_SOUND_PACKET_SAMPLES
            )
                .prop_map(|(codec, rate, channels, samples)| {
                    SourceMessage::Sound(Sound {
                        codec,
                        rate,
                        channels,
                        samples,
                    })
                }),
            prop_oneof![
                Just(SilentReason::Window),
                Just(SilentReason::NoPermission),
                Just(SilentReason::Failed),
                Just(SilentReason::Other)
            ]
            .prop_map(|reason| SourceMessage::Silent(Silent { reason })),
        ]
    }

    fn viewer_message() -> impl Strategy<Value = ViewerMessage> {
        prop_oneof![
            (
                proptest::option::of(any::<u64>()),
                any::<u32>(),
                any::<u64>(),
                thermal()
            )
                .prop_map(|(shown, decode_us, dropped, thermal)| {
                    ViewerMessage::Report(Report {
                        shown,
                        decode_us,
                        dropped,
                        thermal,
                    })
                }),
            (
                1..=MAX_PICTURE_EDGE,
                1..=MAX_PICTURE_EDGE,
                1..=MAX_FRAME_RATE
            )
                .prop_map(|(width, height, fps)| ViewerMessage::Want(Want {
                    width,
                    height,
                    fps
                })),
            any::<u64>().prop_map(|tried| ViewerMessage::KeyFrame(KeyFrame { tried })),
            proptest::collection::vec(event(), 1..=input::MAX_BATCH).prop_map(batch),
            any::<bool>().prop_map(|on| ViewerMessage::Hear(Hear { on })),
        ]
    }

    /// `payload` with one byte replaced, at the position `at` modulo the length.
    fn damaged(payload: &[u8], at: usize, byte: u8) -> Vec<u8> {
        let mut out = payload.to_vec();
        let at = at % out.len();
        out[at] = byte;
        out
    }

    proptest! {
        #[test]
        fn what_is_valid_goes_through_a_frame_and_back(
            source in source_message(),
            viewer in viewer_message(),
        ) {
            let frame = frame::encode(&source).unwrap();
            prop_assert!(frame.len() - frame::HEADER_LEN <= MAX_CONTROL_MESSAGE_BYTES);
            prop_assert_eq!(frame::decode::<SourceMessage>(&frame).unwrap(), source);

            let frame = frame::encode(&viewer).unwrap();
            prop_assert!(frame.len() - frame::HEADER_LEN <= MAX_CONTROL_MESSAGE_BYTES);
            prop_assert_eq!(frame::decode::<ViewerMessage>(&frame).unwrap(), viewer);
        }

        #[test]
        fn what_is_damaged_is_an_error_or_a_message_that_is_valid_and_stable(
            source in source_message(),
            viewer in viewer_message(),
            at in any::<usize>(),
            byte in any::<u8>(),
        ) {
            let payload = &frame::encode(&source).unwrap()[frame::HEADER_LEN..];
            if let Ok(message) = frame::decode_payload::<SourceMessage>(&damaged(payload, at, byte)) {
                let again = frame::encode(&message).unwrap();
                prop_assert_eq!(frame::decode::<SourceMessage>(&again).unwrap(), message);
            }
            let payload = &frame::encode(&viewer).unwrap()[frame::HEADER_LEN..];
            if let Ok(message) = frame::decode_payload::<ViewerMessage>(&damaged(payload, at, byte)) {
                let again = frame::encode(&message).unwrap();
                prop_assert_eq!(frame::decode::<ViewerMessage>(&again).unwrap(), message);
            }
        }

        #[test]
        fn bytes_that_are_not_a_message_are_an_error_and_not_a_panic(
            bytes in proptest::collection::vec(any::<u8>(), 0..512),
        ) {
            if let Ok(message) = frame::decode_payload::<ViewerMessage>(&bytes) {
                prop_assert!(message.validate().is_ok());
            }
            if let Ok(message) = frame::decode_payload::<SourceMessage>(&bytes) {
                prop_assert!(message.validate().is_ok());
            }
        }
    }
}
