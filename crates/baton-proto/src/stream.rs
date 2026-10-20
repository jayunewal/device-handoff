// SPDX-License-Identifier: Apache-2.0

//! Window streaming: the requests that offer a window and end the session.
//!
//! The design is recorded in `docs/adr/0009-window-streaming.md` and specified in section 9 of
//! `docs/protocol.md`. This module holds the messages that travel on ordinary request streams:
//! the source, which is the device whose window it is, makes an [`Offer`]; the viewer, or the
//! person using it, answers once; and either side later ends the session with [`Stop`]. What
//! travels after a yes, the frames of video and the control messages, is not here.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    id::SessionId,
    limits::{
        LimitError, MAX_CODECS, MAX_FRAME_RATE, MAX_NAME_BYTES, MAX_PICTURE_EDGE,
        MAX_STREAM_TITLE_BYTES, MAX_WINDOW_EDGE, check_len,
    },
    message::DeclineReason,
};

/// A request about a streaming session.
///
/// It travels on a bidirectional stream like any other request. An [`Offer`] goes only to a
/// device that advertised [`capability::STREAM_VIEW`](crate::message::capability::STREAM_VIEW).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamRequest {
    /// Ask whether the receiver will show a window.
    Offer(Offer),
    /// End the session, or withdraw an offer that has not been answered.
    Stop(Stop),
}

impl StreamRequest {
    /// The session the request is about.
    pub const fn session(&self) -> SessionId {
        match self {
            Self::Offer(offer) => offer.session,
            Self::Stop(stop) => stop.session,
        }
    }
}

/// The answer to a [`StreamRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamReply {
    /// Answers [`StreamRequest::Offer`]: the receiver will show the window.
    Accepted(Accepted),
    /// Answers [`StreamRequest::Offer`]: the receiver will not. A device that takes no part in
    /// streaming answers every request this way, with
    /// [`Unsupported`](DeclineReason::Unsupported).
    Declined(Declined),
    /// Answers [`StreamRequest::Stop`].
    Stopped(Stopped),
}

impl StreamReply {
    /// The session the reply is about.
    pub const fn session(&self) -> SessionId {
        match self {
            Self::Accepted(accepted) => accepted.session,
            Self::Declined(declined) => declined.session,
            Self::Stopped(stopped) => stopped.session,
        }
    }
}

/// What kind of thing the source shares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    /// One window of one application.
    Window,
    /// One whole display, with every window on it. It has no application, so `app` is empty, and
    /// its `title` is its name, if the source knows one.
    Display,
    /// Any kind this version does not know about. A receiver declines it as unsupported.
    #[serde(other)]
    Other,
}

/// How the picture is coded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec {
    /// H.264 in Annex B form, with the parameter sets in front of every key frame.
    H264,
    /// Any codec this version does not know about.
    #[serde(other)]
    Other,
}

/// Why a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    /// The person stopped sharing or viewing.
    Ended,
    /// The window that was shared is gone.
    WindowClosed,
    /// Something went wrong that the person did not choose, such as the permission for
    /// capture being taken away.
    Failed,
    /// Any reason this version does not know about.
    #[serde(other)]
    Other,
}

/// The source offers to show a window, or a whole display.
///
/// This is all the person at the viewer is shown before they decide, so it says what is shared
/// and whether the viewer may control it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    /// Chosen at random by the source for each offer and never used again.
    pub session: SessionId,
    /// What is shared.
    pub source: SourceKind,
    /// The title of the window, or the name of the display. Text that another device wrote.
    pub title: String,
    /// The name of the application the window belongs to, empty for a display. Text that another
    /// device wrote.
    pub app: String,
    /// The width of the window or the display in pixels.
    pub width: u32,
    /// The height of the window or the display in pixels.
    pub height: u32,
    /// Whether the person at the source granted the viewer control of the window. The viewer
    /// cannot ask for it and cannot turn it on later. Left out when it is not granted.
    #[serde(default, skip_serializing_if = "is_default")]
    pub control: bool,
}

// The title of a window can name a document, a person or a page, and the application tells
// what the person is doing. Neither goes into a log.
impl fmt::Debug for Offer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Offer")
            .field("session", &self.session)
            .field("source", &self.source)
            .field("title_bytes", &self.title.len())
            .field("app_bytes", &self.app.len())
            .field("width", &self.width)
            .field("height", &self.height)
            .field("control", &self.control)
            .finish()
    }
}

/// The receiver will show the window, and says what it can take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accepted {
    /// The session of the offer this answers.
    pub session: SessionId,
    /// The widest picture the viewer can show, in pixels.
    pub max_width: u32,
    /// The highest picture the viewer can show, in pixels.
    pub max_height: u32,
    /// The most frames a second it can decode and show.
    pub max_fps: u16,
    /// The codecs it can decode, at least one.
    pub codecs: Vec<Codec>,
}

/// The receiver will not show the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Declined {
    /// The session of the offer this answers.
    pub session: SessionId,
    /// Why not.
    pub reason: DeclineReason,
}

/// One side ends the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stop {
    /// The session that ends.
    pub session: SessionId,
    /// Why.
    pub reason: StopReason,
}

/// The receiver has dropped the session and what it held for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stopped {
    /// The session that ended. A receiver that never had it, or has forgotten it, answers the
    /// same way: there is nothing left to stop.
    pub session: SessionId,
}

/// For `skip_serializing_if`: a field that has its default (`false`) is left out of the
/// message, and a reader that finds it absent takes the same value.
fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

fn invalid(field: &'static str, reason: &'static str) -> LimitError {
    LimitError::Invalid { field, reason }
}

fn check_edge(field: &'static str, value: u32, max: u32) -> Result<(), LimitError> {
    if (1..=max).contains(&value) {
        Ok(())
    } else {
        Err(invalid(field, "is not between 1 and the most there may be"))
    }
}

impl Offer {
    fn validate(&self) -> Result<(), LimitError> {
        check_len("window title", &self.title, MAX_STREAM_TITLE_BYTES)?;
        check_len("application name", &self.app, MAX_NAME_BYTES)?;
        check_edge("window width", self.width, MAX_WINDOW_EDGE)?;
        check_edge("window height", self.height, MAX_WINDOW_EDGE)
    }
}

impl Accepted {
    fn validate(&self) -> Result<(), LimitError> {
        check_edge("largest picture width", self.max_width, MAX_PICTURE_EDGE)?;
        check_edge("largest picture height", self.max_height, MAX_PICTURE_EDGE)?;
        if !(1..=MAX_FRAME_RATE).contains(&self.max_fps) {
            return Err(invalid(
                "highest frame rate",
                "is not between 1 and the most there may be",
            ));
        }
        if self.codecs.is_empty() {
            return Err(LimitError::Empty { field: "codecs" });
        }
        if self.codecs.len() > MAX_CODECS {
            return Err(LimitError::TooMany {
                field: "codecs",
                count: self.codecs.len(),
                max: MAX_CODECS,
            });
        }
        Ok(())
    }
}

impl StreamRequest {
    pub(crate) fn validate(&self) -> Result<(), LimitError> {
        match self {
            Self::Offer(offer) => offer.validate(),
            Self::Stop(_) => Ok(()),
        }
    }
}

impl StreamReply {
    pub(crate) fn validate(&self) -> Result<(), LimitError> {
        match self {
            Self::Accepted(accepted) => accepted.validate(),
            Self::Declined(_) | Self::Stopped(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        frame,
        message::{Request, Response},
    };

    fn session() -> SessionId {
        SessionId::from_bytes(std::array::from_fn(|i| u8::try_from(i + 0xa0).unwrap()))
    }

    fn offer() -> Offer {
        Offer {
            session: session(),
            source: SourceKind::Window,
            title: "Report.pdf".into(),
            app: "Preview".into(),
            width: 1440,
            height: 900,
            control: true,
        }
    }

    fn accepted() -> Accepted {
        Accepted {
            session: session(),
            max_width: 2732,
            max_height: 2048,
            max_fps: 60,
            codecs: vec![Codec::H264],
        }
    }

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write;
        bytes.iter().fold(String::new(), |mut out, b| {
            write!(out, "{b:02x}").unwrap();
            out
        })
    }

    #[test]
    fn the_messages_of_a_session_are_read_back_as_they_were_written() {
        let requests = [
            StreamRequest::Offer(offer()),
            StreamRequest::Offer(Offer {
                control: false,
                ..offer()
            }),
            StreamRequest::Stop(Stop {
                session: session(),
                reason: StopReason::WindowClosed,
            }),
        ];
        for request in requests {
            let sent = Request::Stream(request);
            let read = frame::decode::<Request>(&frame::encode(&sent).unwrap()).unwrap();
            assert_eq!(read, sent);
        }
        let replies = [
            StreamReply::Accepted(accepted()),
            StreamReply::Declined(Declined {
                session: session(),
                reason: DeclineReason::Refused,
            }),
            StreamReply::Stopped(Stopped { session: session() }),
        ];
        for reply in replies {
            let sent = Response::Stream(reply);
            let read = frame::decode::<Response>(&frame::encode(&sent).unwrap()).unwrap();
            assert_eq!(read, sent);
        }
    }

    #[test]
    fn the_vectors_of_protocol_md_are_what_the_code_writes() {
        // The exact bytes are the protocol (docs/protocol.md section 7). If this fails, the
        // wire format changed.
        let controlled = frame::encode(&Request::Stream(StreamRequest::Offer(offer()))).unwrap();
        assert_eq!(
            hex(&controlled),
            concat!(
                "00000070",                           // the frame: 112 bytes follow
                "a16653747265616d",                   // {"Stream":
                "a1654f66666572",                     //   {"Offer":
                "a7",                                 //     map of 7
                "6773657373696f6e",                   //     "session":
                "50a0a1a2a3a4a5a6a7a8a9aaabacadaeaf", //       16 bytes
                "66736f75726365",                     //     "source":
                "6657696e646f77",                     //       "Window",
                "657469746c65",                       //     "title":
                "6a5265706f72742e706466",             //       "Report.pdf",
                "63617070",                           //     "app":
                "6750726576696577",                   //       "Preview",
                "6577696474681905a0",                 //     "width": 1440,
                "66686569676874190384",               //     "height": 900,
                "67636f6e74726f6c",                   //     "control":
                "f5",                                 //       true
            )
        );

        // Without control the key is left out, which is nine bytes less.
        let view_only = frame::encode(&Request::Stream(StreamRequest::Offer(Offer {
            control: false,
            ..offer()
        })))
        .unwrap();
        assert_eq!(view_only.len(), controlled.len() - 9);
        assert_eq!(view_only.get(..4), Some(&[0, 0, 0, 0x67][..]));

        let accepted = frame::encode(&Response::Stream(StreamReply::Accepted(accepted()))).unwrap();
        assert_eq!(
            hex(&accepted),
            concat!(
                "0000005e",                           // the frame: 94 bytes follow
                "a16653747265616d",                   // {"Stream":
                "a1684163636570746564",               //   {"Accepted":
                "a5",                                 //     map of 5
                "6773657373696f6e",                   //     "session":
                "50a0a1a2a3a4a5a6a7a8a9aaabacadaeaf", //   16 bytes
                "696d61785f7769647468190aac",         //     "max_width": 2732,
                "6a6d61785f686569676874190800",       //     "max_height": 2048,
                "676d61785f667073183c",               //     "max_fps": 60,
                "66636f6465637381",                   //     "codecs": [
                "6448323634",                         //       "H264"]
            )
        );
    }

    #[test]
    fn the_session_of_a_request_and_of_a_reply_is_the_one_they_name() {
        let other = SessionId::from_bytes([9; 16]);
        assert_eq!(StreamRequest::Offer(offer()).session(), session());
        assert_eq!(
            StreamRequest::Stop(Stop {
                session: other,
                reason: StopReason::Ended
            })
            .session(),
            other
        );
        assert_eq!(StreamReply::Accepted(accepted()).session(), session());
        assert_eq!(
            StreamReply::Declined(Declined {
                session: other,
                reason: DeclineReason::Blocked
            })
            .session(),
            other
        );
        assert_eq!(
            StreamReply::Stopped(Stopped { session: other }).session(),
            other
        );
    }

    #[test]
    fn the_limits_on_an_offer_are_enforced() {
        let check = |offer: Offer| StreamRequest::Offer(offer).validate();
        assert_eq!(check(offer()), Ok(()));
        // A window may have no title, and an application no name.
        assert_eq!(
            check(Offer {
                title: String::new(),
                app: String::new(),
                ..offer()
            }),
            Ok(())
        );
        assert_eq!(
            check(Offer {
                title: "t".repeat(MAX_STREAM_TITLE_BYTES),
                app: "a".repeat(MAX_NAME_BYTES),
                width: MAX_WINDOW_EDGE,
                height: MAX_WINDOW_EDGE,
                ..offer()
            }),
            Ok(())
        );
        for bad in [
            Offer {
                title: "t".repeat(MAX_STREAM_TITLE_BYTES + 1),
                ..offer()
            },
            Offer {
                app: "a".repeat(MAX_NAME_BYTES + 1),
                ..offer()
            },
            Offer {
                width: 0,
                ..offer()
            },
            Offer {
                height: 0,
                ..offer()
            },
            Offer {
                width: MAX_WINDOW_EDGE + 1,
                ..offer()
            },
            Offer {
                height: u32::MAX,
                ..offer()
            },
        ] {
            assert!(check(bad.clone()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_limits_on_what_a_viewer_can_take_are_enforced() {
        let check = |accepted: Accepted| StreamReply::Accepted(accepted).validate();
        assert_eq!(check(accepted()), Ok(()));
        assert_eq!(
            check(Accepted {
                max_width: MAX_PICTURE_EDGE,
                max_height: MAX_PICTURE_EDGE,
                max_fps: MAX_FRAME_RATE,
                codecs: vec![Codec::H264; MAX_CODECS],
                ..accepted()
            }),
            Ok(())
        );
        for bad in [
            Accepted {
                max_width: 0,
                ..accepted()
            },
            Accepted {
                max_height: 0,
                ..accepted()
            },
            Accepted {
                max_width: MAX_PICTURE_EDGE + 1,
                ..accepted()
            },
            Accepted {
                max_height: MAX_PICTURE_EDGE + 1,
                ..accepted()
            },
            Accepted {
                max_fps: 0,
                ..accepted()
            },
            Accepted {
                max_fps: MAX_FRAME_RATE + 1,
                ..accepted()
            },
            Accepted {
                codecs: Vec::new(),
                ..accepted()
            },
            Accepted {
                codecs: vec![Codec::H264; MAX_CODECS + 1],
                ..accepted()
            },
        ] {
            assert!(check(bad.clone()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_title_and_the_application_stay_out_of_a_log() {
        let shown = format!(
            "{:?}",
            Offer {
                title: "Salary 2026.xlsx".into(),
                app: "Numbers".into(),
                ..offer()
            }
        );
        assert!(!shown.contains("Salary"), "{shown}");
        assert!(!shown.contains("Numbers"), "{shown}");
        assert!(shown.contains("title_bytes: 16"), "{shown}");
        assert!(shown.contains("app_bytes: 7"), "{shown}");
    }
}
