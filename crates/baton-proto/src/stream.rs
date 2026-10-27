// SPDX-License-Identifier: Apache-2.0

//! Window streaming: the requests that offer a window, start a session and end it.
//!
//! The design is recorded in `docs/adr/0009-window-streaming.md` and
//! `docs/adr/0010-sessions-that-the-viewer-starts.md` and specified in section 9 of
//! `docs/protocol.md`. This module holds the messages that travel on ordinary request streams:
//! the source, which is the device whose window it is, makes an [`Offer`]; the viewer, or the
//! person using it, answers once; and either side later ends the session with [`Stop`]. A viewer
//! may also ask a source to make the offer, with a [`Start`], when the person at the source has
//! allowed that beforehand. What travels after a yes, the frames of video and the control
//! messages, is not here.

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
/// device that advertised [`capability::STREAM_VIEW`](crate::message::capability::STREAM_VIEW),
/// and a [`Start`] only to one that advertised
/// [`capability::STREAM_START`](crate::message::capability::STREAM_START).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamRequest {
    /// Ask whether the receiver will show a window.
    Offer(Offer),
    /// End the session, or withdraw an offer that has not been answered, or take back a start
    /// whose offer has not been made.
    Stop(Stop),
    /// Ask the receiver to make an offer of something of its own to the sender.
    Start(Start),
}

impl StreamRequest {
    /// The session the request is about.
    pub const fn session(&self) -> SessionId {
        match self {
            Self::Offer(offer) => offer.session,
            Self::Stop(stop) => stop.session,
            Self::Start(start) => start.session,
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
    /// Answers [`StreamRequest::Start`]: the receiver will make the offer.
    Starting(Starting),
    /// Answers [`StreamRequest::Start`]: the receiver will not.
    NotStarted(NotStarted),
}

impl StreamReply {
    /// The session the reply is about.
    pub const fn session(&self) -> SessionId {
        match self {
            Self::Accepted(accepted) => accepted.session,
            Self::Declined(declined) => declined.session,
            Self::Stopped(stopped) => stopped.session,
            Self::Starting(starting) => starting.session,
            Self::NotStarted(not_started) => not_started.session,
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
#[allow(clippy::struct_excessive_bools)] // what the source says of the session, each of its own
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
    /// Whether the source keeps its own screen dark and its keys and pointer out of use for as
    /// long as the session runs, until the viewer asks it to lift either with a `Lift` on the
    /// control stream. Only a source that advertised `stream.private` sets it. Left out when the
    /// screen is not kept private.
    #[serde(default, skip_serializing_if = "is_default")]
    pub private: bool,
    /// Whether the person at the source set up the unlock of its screen for the viewer, so that
    /// the viewer may ask the source to unlock it when it is locked, with an `Unlock` on the
    /// control stream. Only a source that advertised `unlock.target` sets it, to a viewer that
    /// advertised `unlock.approve`, and only with `private`. The viewer cannot turn it on later.
    /// Left out when the viewer may not.
    #[serde(default, skip_serializing_if = "is_default")]
    pub unlock: bool,
    /// Whether the source will leave its screen as it is when the session ends, and not lock it,
    /// if the person at the viewer chose that and the viewer says so with a `Leave` on the control
    /// stream before it finishes the stream. Only a source whose platform can end the privacy of a
    /// session without locking sets it, and only with `private`. The viewer cannot turn it on
    /// later. Left out when the source will not.
    #[serde(default, skip_serializing_if = "is_default")]
    pub leave: bool,
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
            .field("private", &self.private)
            .field("unlock", &self.unlock)
            .field("leave", &self.leave)
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

/// What a viewer wants the source to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Wanted {
    /// The main display of the source, with every window on it: the screen that the person at the
    /// source is looking at. The source chooses which display that is.
    Screen,
    /// Anything this version does not know about. A source answers it with
    /// [`Unsupported`](WhyNot::Unsupported).
    #[serde(other)]
    Other,
}

/// The viewer asks the source to make an offer, so that the person at the viewer does not have to
/// go to the source to share something. The source does so only for a device that the person at
/// the source allowed it for beforehand; there is no question to a person then.
///
/// What the viewer wants to see is all this says. The offer that answers it is made under the same
/// session, and says what is shared and whether the viewer may control it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Start {
    /// Chosen at random by the viewer for each start and never used again. The source makes its
    /// offer under this id.
    pub session: SessionId,
    /// What the viewer wants to see.
    pub wanted: Wanted,
    /// Whether the viewer would like to control what it sees. It is a wish and no more: the
    /// person at the source decided beforehand whether this device may, and the offer says what
    /// was granted. Left out when it is not wished.
    #[serde(default, skip_serializing_if = "is_default")]
    pub control: bool,
}

/// The source will make the offer for a start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Starting {
    /// The session of the start this answers.
    pub session: SessionId,
}

/// The source will not make an offer for a start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotStarted {
    /// The session of the start this answers.
    pub session: SessionId,
    /// Why not.
    pub why: WhyNot,
}

/// Why a source did not start a session when it was asked to.
///
/// Each is something a person can be told, and most of them something that the person at the
/// source has to put right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WhyNot {
    /// This device cannot start a session, or not for what was asked.
    Unsupported,
    /// The settings at the source do not let this device start a session. Only the person at the
    /// source can change that.
    NotAllowed,
    /// The source is offering or sharing something already. It shares one thing at a time.
    Busy,
    /// The viewer asked too often.
    RateLimited,
    /// The request is not one a source can act on, for example the id was used before.
    Invalid,
    /// There is nothing to show now, for example the source has no display on.
    Unavailable,
    /// The operating system of the source has not allowed its program to capture the screen, and
    /// the person at the source has to allow it.
    NeedsPermission,
    /// The operating system of the source has not allowed its program to hold the keyboard and the
    /// pointer of the source, which the source does while a device controls it so that nobody at
    /// the source can use it, and the person at the source has to allow it.
    NeedsInputPermission,
    /// Something went wrong at the source that the viewer did not cause, or it did not get ready
    /// in time.
    Failed,
    /// Any reason this version does not know about.
    #[serde(other)]
    Other,
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
            Self::Stop(_) | Self::Start(_) => Ok(()),
        }
    }
}

impl StreamReply {
    pub(crate) fn validate(&self) -> Result<(), LimitError> {
        match self {
            Self::Accepted(accepted) => accepted.validate(),
            Self::Declined(_) | Self::Stopped(_) | Self::Starting(_) | Self::NotStarted(_) => {
                Ok(())
            }
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
            private: false,
            unlock: false,
            leave: false,
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

    fn start() -> Start {
        Start {
            session: session(),
            wanted: Wanted::Screen,
            control: true,
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
            StreamRequest::Offer(Offer {
                private: true,
                ..offer()
            }),
            StreamRequest::Offer(Offer {
                private: true,
                unlock: true,
                ..offer()
            }),
            StreamRequest::Offer(Offer {
                private: true,
                leave: true,
                ..offer()
            }),
            StreamRequest::Offer(Offer {
                private: true,
                unlock: true,
                leave: true,
                ..offer()
            }),
            StreamRequest::Stop(Stop {
                session: session(),
                reason: StopReason::WindowClosed,
            }),
            StreamRequest::Start(start()),
            StreamRequest::Start(Start {
                control: false,
                ..start()
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
            StreamReply::Starting(Starting { session: session() }),
            StreamReply::NotStarted(NotStarted {
                session: session(),
                why: WhyNot::NotAllowed,
            }),
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

        // A private session has one more key behind `control`, which is nine bytes more.
        let private = frame::encode(&Request::Stream(StreamRequest::Offer(Offer {
            private: true,
            ..offer()
        })))
        .unwrap();
        assert_eq!(
            hex(&private),
            concat!(
                "00000079",                           // the frame: 121 bytes follow
                "a16653747265616d",                   // {"Stream":
                "a1654f66666572",                     //   {"Offer":
                "a8",                                 //     map of 8
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
                "f5",                                 //       true,
                "6770726976617465",                   //     "private":
                "f5",                                 //       true
            )
        );

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
    fn the_vector_of_an_offer_that_may_be_unlocked_is_what_the_code_writes() {
        let private = frame::encode(&Request::Stream(StreamRequest::Offer(Offer {
            private: true,
            ..offer()
        })))
        .unwrap();
        // A session that the viewer may unlock has one more key behind `private`, which is eight
        // bytes more.
        let unlocking = frame::encode(&Request::Stream(StreamRequest::Offer(Offer {
            private: true,
            unlock: true,
            ..offer()
        })))
        .unwrap();
        assert_eq!(
            hex(&unlocking),
            concat!(
                "00000081",                           // the frame: 129 bytes follow
                "a16653747265616d",                   // {"Stream":
                "a1654f66666572",                     //   {"Offer":
                "a9",                                 //     map of 9
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
                "f5",                                 //       true,
                "6770726976617465",                   //     "private":
                "f5",                                 //       true,
                "66756e6c6f636b",                     //     "unlock":
                "f5",                                 //       true
            )
        );
        assert_eq!(unlocking.len(), private.len() + 8);
    }

    #[test]
    fn the_vectors_of_an_offer_that_may_be_left_are_what_the_code_writes() {
        let private = frame::encode(&Request::Stream(StreamRequest::Offer(Offer {
            private: true,
            ..offer()
        })))
        .unwrap();
        // A source that will leave its screen as it is has one more key behind `private`, which
        // is seven bytes more.
        let leaving = frame::encode(&Request::Stream(StreamRequest::Offer(Offer {
            private: true,
            leave: true,
            ..offer()
        })))
        .unwrap();
        assert_eq!(
            hex(&leaving),
            concat!(
                "00000080",                           // the frame: 128 bytes follow
                "a16653747265616d",                   // {"Stream":
                "a1654f66666572",                     //   {"Offer":
                "a9",                                 //     map of 9
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
                "f5",                                 //       true,
                "6770726976617465",                   //     "private":
                "f5",                                 //       true,
                "656c65617665",                       //     "leave":
                "f5",                                 //       true
            )
        );
        assert_eq!(leaving.len(), private.len() + 7);

        // And one that may be unlocked as well has `leave` behind `unlock`.
        let both = frame::encode(&Request::Stream(StreamRequest::Offer(Offer {
            private: true,
            unlock: true,
            leave: true,
            ..offer()
        })))
        .unwrap();
        assert_eq!(
            hex(&both[both.len() - 15..]),
            concat!(
                "66756e6c6f636b", // "unlock":
                "f5",             //   true,
                "656c65617665",   // "leave":
                "f5",             //   true
            )
        );
        assert_eq!(both.len(), private.len() + 8 + 7);
    }

    #[test]
    fn the_vectors_of_a_start_are_what_the_code_writes() {
        let wished = frame::encode(&Request::Stream(StreamRequest::Start(start()))).unwrap();
        assert_eq!(
            hex(&wished),
            concat!(
                "00000040",                           // the frame: 64 bytes follow
                "a16653747265616d",                   // {"Stream":
                "a1655374617274",                     //   {"Start":
                "a3",                                 //     map of 3
                "6773657373696f6e",                   //     "session":
                "50a0a1a2a3a4a5a6a7a8a9aaabacadaeaf", //       16 bytes
                "6677616e746564",                     //     "wanted":
                "6653637265656e",                     //       "Screen",
                "67636f6e74726f6c",                   //     "control":
                "f5",                                 //       true
            )
        );

        // Without the wish for control the key is left out, which is nine bytes less.
        let looking = frame::encode(&Request::Stream(StreamRequest::Start(Start {
            control: false,
            ..start()
        })))
        .unwrap();
        assert_eq!(looking.len(), wished.len() - 9);
        assert_eq!(looking.get(..4), Some(&[0, 0, 0, 0x37][..]));
        assert_eq!(looking.get(19..20), Some(&[0xa2][..]), "a map of 2");

        let starting = frame::encode(&Response::Stream(StreamReply::Starting(Starting {
            session: session(),
        })))
        .unwrap();
        assert_eq!(
            hex(&starting),
            concat!(
                "0000002c",                           // the frame: 44 bytes follow
                "a16653747265616d",                   // {"Stream":
                "a1685374617274696e67",               //   {"Starting":
                "a1",                                 //     map of 1
                "6773657373696f6e",                   //     "session":
                "50a0a1a2a3a4a5a6a7a8a9aaabacadaeaf", //       16 bytes
            )
        );

        let not_started = frame::encode(&Response::Stream(StreamReply::NotStarted(NotStarted {
            session: session(),
            why: WhyNot::NotAllowed,
        })))
        .unwrap();
        assert_eq!(
            hex(&not_started),
            concat!(
                "0000003d",                           // the frame: 61 bytes follow
                "a16653747265616d",                   // {"Stream":
                "a16a4e6f7453746172746564",           //   {"NotStarted":
                "a2",                                 //     map of 2
                "6773657373696f6e",                   //     "session":
                "50a0a1a2a3a4a5a6a7a8a9aaabacadaeaf", //       16 bytes
                "63776879",                           //     "why":
                "6a4e6f74416c6c6f776564",             //       "NotAllowed"
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
        assert_eq!(
            StreamRequest::Start(Start {
                session: other,
                ..start()
            })
            .session(),
            other
        );
        assert_eq!(
            StreamReply::Starting(Starting { session: other }).session(),
            other
        );
        assert_eq!(
            StreamReply::NotStarted(NotStarted {
                session: other,
                why: WhyNot::Busy
            })
            .session(),
            other
        );
    }

    #[test]
    fn a_start_and_what_answers_it_have_no_limits_to_break() {
        // There is no text in them, and the numbers have no range.
        for wanted in [Wanted::Screen, Wanted::Other] {
            for control in [false, true] {
                let start = Start {
                    wanted,
                    control,
                    ..start()
                };
                assert_eq!(StreamRequest::Start(start).validate(), Ok(()));
            }
        }
        assert_eq!(
            StreamReply::Starting(Starting { session: session() }).validate(),
            Ok(())
        );
        assert_eq!(
            StreamReply::NotStarted(NotStarted {
                session: session(),
                why: WhyNot::Other
            })
            .validate(),
            Ok(())
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

    #[test]
    fn a_log_says_whether_the_viewer_may_unlock() {
        let shown = format!(
            "{:?}",
            Offer {
                private: true,
                unlock: true,
                ..offer()
            }
        );
        assert!(shown.contains("unlock: true"), "{shown}");
        assert!(format!("{:?}", offer()).contains("unlock: false"));
    }

    #[test]
    fn a_log_says_whether_the_source_will_leave_its_screen_as_it_is() {
        let shown = format!(
            "{:?}",
            Offer {
                private: true,
                leave: true,
                ..offer()
            }
        );
        assert!(shown.contains("leave: true"), "{shown}");
        assert!(format!("{:?}", offer()).contains("leave: false"));
    }
}
