// SPDX-License-Identifier: Apache-2.0

//! The sharer that the platform registered, as the app's [`WindowSharer`], the windows that the
//! person picks with it, as the app's [`PickedWindow`], and what the platform may do with a window
//! that runs, through its link.
//!
//! The platform's functions for one window are called in this order, one after the other and
//! never at the same time, because one owner holds the window and passes it on:
//!
//! 1. [`AbiSharer::pick`] asks the platform to show its picker. The answer comes later, from any
//!    thread, with [`Picks::answer`], and gives the window to an [`AbiPicked`], which holds the
//!    platform's pointer for it from then on and releases it when it is dropped.
//! 2. `prepare`, once the other device has said yes: the platform chooses the picture to begin
//!    with, and what it chose is held to the protocol and to what the other device takes.
//! 3. `start`, when the session runs. The window gets a link in the table of [`Links`] first, so
//!    that the platform can use it from the first moment, and a task waits for the end.
//! 4. `ended`, from that task, once the session is over, whoever ended it, and then `release`
//!    when the window is dropped. Neither is called with a lock of the session held, so neither
//!    can wait for a thread of the platform that is on its way into the session.
//!
//! A window that never starts (the other device said no, or `prepare` failed) is released when
//! its owner is dropped, without `ended`, which only follows a `start` that worked.

use std::{
    collections::BTreeMap,
    ffi::c_void,
    ptr::NonNull,
    sync::{Arc, Mutex},
    task::{Context, Waker},
    time::Duration,
};

use baton_app::windows::{PickedWindow, PlatformError, SharingFailure, WindowSharer};
use baton_core::{
    streaming::{NotSent, Picture, SourceEvents, SourceHandle, Takes},
    util::BoxFuture,
};
use baton_proto::{
    limits::{MAX_FRAME_RATE, MAX_PICTURE_EDGE},
    stream::Codec,
};
use baton_stream::{
    control::{Config as Format, InputBatch, PauseReason, Report, Want},
    h264,
    header::FrameKind,
    sender::LtrToken,
};
use tokio::sync::oneshot;

use crate::{
    Error,
    abi::{
        BEGIN_DELTA, BEGIN_KEY, BEGIN_NOT_NOW, BEGIN_RECOVERY, CODEC_H264, PAUSE_HIDDEN,
        PAUSE_LOCKED, PAUSE_OTHER, SENT, SENT_OVER, SENT_STALE, SENT_TOO_LARGE,
    },
    platform::{Chosen, PickedHandle, SharerCalls, Takes as WhatItTakes},
    registry::{Links, lock},
};

/// What the person is told when the platform could not show its picker and gave no reason.
const NO_PICKER: &str = "This device could not show the list of windows to choose from.";

/// What the person is told when the platform's answer to a pick was not one that can be used.
const NOT_CHOSEN: &str = "This device could not tell which window was chosen.";

/// What the person is told when nothing answered a pick before the library went away.
const NO_ANSWER: &str = "The choice of a window was lost.";

/// What the person is told when the platform could not start to capture and gave no reason.
const NOT_STARTED: &str = "This device could not start sharing the window.";

/// What the platform shares with the windows that it picks.
pub(crate) struct Shared {
    calls: SharerCalls,
    links: Arc<Links>,
}

/// The window that the platform gave with its answer to a pick, as it was read.
pub(crate) struct Given {
    /// The platform's pointer for the window, not null.
    pub(crate) context: NonNull<c_void>,
    /// Whether it is a whole display and not one window.
    pub(crate) display: bool,
    pub(crate) title: String,
    pub(crate) app: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// The answer of the platform to a pick.
pub(crate) enum PickAnswer {
    /// The person chose a window.
    Chosen(Given),
    /// The picker failed, and this is what the person is told.
    Failed(String),
    /// The person gave up.
    GaveUp,
}

/// Who waits for the answer to a pick.
struct Waiting {
    reply: oneshot::Sender<Result<Option<AbiPicked>, PlatformError>>,
    shared: Arc<Shared>,
}

#[derive(Default)]
struct PickTable {
    last: u64,
    waiting: BTreeMap<u64, Waiting>,
}

/// The picks that wait for an answer, by the number that the platform was given for each.
///
/// A number is never used twice and is not 0, so a platform that answers twice, or after nobody
/// waits, or with a number it made up, is told so and nothing else happens.
#[derive(Default)]
pub(crate) struct Picks {
    table: Mutex<PickTable>,
}

impl Picks {
    fn add(&self, waiting: Waiting) -> u64 {
        let mut table = lock(&self.table);
        table.last = table.last.saturating_add(1);
        let request = table.last;
        table.waiting.insert(request, waiting);
        request
    }

    fn take(&self, request: u64) -> Option<Waiting> {
        lock(&self.table).waiting.remove(&request)
    }

    /// How many picks wait.
    pub(crate) fn len(&self) -> usize {
        lock(&self.table).waiting.len()
    }

    /// Gives the platform's answer to the pick that waits for it, once.
    ///
    /// An answer that could not be read (`Err`) still ends the wait, with a sentence for the
    /// person: somebody is looking at the screen for the window they asked for. The window is
    /// not taken then, and stays with the platform.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownLink`] if nobody waits for `request`, and the error of an answer that
    /// could not be read.
    pub(crate) fn answer(
        &self,
        request: u64,
        answer: Result<PickAnswer, Error>,
    ) -> Result<(), Error> {
        let waiting = self.take(request).ok_or(Error::UnknownLink)?;
        let (result, status) = match answer {
            Ok(PickAnswer::Chosen(given)) => {
                (Ok(Some(AbiPicked::of(given, &waiting.shared))), Ok(()))
            }
            Ok(PickAnswer::Failed(sentence)) => (Err(PlatformError::new(sentence)), Ok(())),
            Ok(PickAnswer::GaveUp) => (Ok(None), Ok(())),
            Err(error) => (Err(PlatformError::new(NOT_CHOSEN)), Err(error)),
        };
        // The future may be gone by now. A window that was in the result is released then, as it
        // is by whoever else drops it, and the platform was told that the Rust side took it.
        drop(waiting.reply.send(result));
        status
    }
}

/// The sharer of the platform.
pub(crate) struct AbiSharer {
    shared: Arc<Shared>,
    picks: Arc<Picks>,
}

impl AbiSharer {
    /// A sharer for the platform that registered `calls`.
    pub(crate) fn new(calls: SharerCalls, links: Arc<Links>, picks: Arc<Picks>) -> Self {
        Self {
            shared: Arc::new(Shared { calls, links }),
            picks,
        }
    }
}

impl WindowSharer for AbiSharer {
    fn control(&self) -> bool {
        self.shared.calls.control()
    }

    fn pick(&self) -> BoxFuture<'static, Result<Option<Box<dyn PickedWindow>>, PlatformError>> {
        let (shared, picks) = (Arc::clone(&self.shared), Arc::clone(&self.picks));
        Box::pin(async move {
            let (reply, answer) = oneshot::channel();
            let request = picks.add(Waiting {
                reply,
                shared: Arc::clone(&shared),
            });
            // From here on, dropping the future takes the request down.
            let _withdraw = Withdraw {
                picks: &picks,
                shared: &shared,
                request,
            };
            if let Err(failure) = shared.calls.pick(request) {
                // The picker was never shown, so there is nothing to take down: the request is
                // gone from the table before the guard looks for it.
                drop(picks.take(request));
                return Err(PlatformError::new(
                    failure.0.unwrap_or_else(|| NO_PICKER.to_owned()),
                ));
            }
            match answer.await {
                Ok(result) => result
                    .map(|picked| picked.map(|picked| Box::new(picked) as Box<dyn PickedWindow>)),
                Err(_) => Err(PlatformError::new(NO_ANSWER)),
            }
        })
    }
}

/// Takes the request of a pick down when the future that waits for it is dropped first: the
/// request leaves the table, so that a late answer is refused, and the platform is told that
/// nobody waits any more.
///
/// A request that is not in the table any more is left alone: an answer took it out, and the
/// picker is gone, or the pick failed and no picker was shown.
struct Withdraw<'a> {
    picks: &'a Picks,
    shared: &'a Shared,
    request: u64,
}

impl Drop for Withdraw<'_> {
    fn drop(&mut self) {
        if self.picks.take(self.request).is_some() {
            self.shared.calls.cancel_pick(self.request);
        }
    }
}

/// A window, or a whole display, that the person picked.
pub(crate) struct AbiPicked {
    window: PickedHandle,
    shared: Arc<Shared>,
    display: bool,
    title: String,
    app: String,
    width: u32,
    height: u32,
    /// The size of the picture that `prepare` chose, once it has.
    prepared: Option<(u32, u32)>,
}

impl AbiPicked {
    fn of(given: Given, shared: &Arc<Shared>) -> Self {
        Self {
            window: shared.calls.adopt(given.context),
            shared: Arc::clone(shared),
            display: given.display,
            title: given.title,
            app: given.app,
            width: given.width,
            height: given.height,
            prepared: None,
        }
    }
}

/// What the session tells the platform that the interface does not carry yet: how the viewer is
/// doing, what it wants, what the person does there, which references it holds, and that a
/// recovery is wanted. The platform asks `begin` and `recovery_pending` on its own clock, and a
/// window is shared without control. The end is heard through `on_end`, outside the lock of the
/// session, see [`AbiPicked::start`].
struct Quiet;

impl SourceEvents for Quiet {
    fn report(&mut self, _: Report) {}

    fn want(&mut self, _: Want) {}

    fn input(&mut self, _: InputBatch, _: Duration) {}

    fn held(&mut self, _: Vec<LtrToken>) {}

    fn recovery(&mut self) {}

    fn ended(&mut self) {}
}

impl PickedWindow for AbiPicked {
    fn display(&self) -> bool {
        self.display
    }

    fn title(&self) -> String {
        self.title.clone()
    }

    fn app(&self) -> String {
        self.app.clone()
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn events(&mut self) -> Box<dyn SourceEvents> {
        Box::new(Quiet)
    }

    fn prepare(&mut self, takes: &Takes) -> Option<Format> {
        let chosen = self.window.prepare(&what_it_takes(takes))?;
        // The session checks the same, but a picture beyond what the other device takes is not
        // one that the platform is to be given the chance to make.
        let format = format_of(&chosen)
            .ok()
            .filter(|format| takes.allows(format))?;
        self.prepared = Some((format.width, format.height));
        Some(format)
    }

    /// Starts the capture. The window is given a link first, so that the platform's threads can
    /// use it from the first moment, and a task waits for the end of the session: when it comes,
    /// whoever ended the session, the link closes, the platform is told with `ended`, and the
    /// window is released. That is done outside the lock of the session, which the events of the
    /// session are called with, so `ended` and `release` may stop threads that are in the middle
    /// of a call into the library.
    fn start(self: Box<Self>, handle: Arc<SourceHandle>, failure: SharingFailure) {
        let Self {
            mut window,
            shared,
            prepared,
            ..
        } = *self;
        let mut over = Box::pin(handle.on_end());
        // A session that is over before it starts does not get a capture that is stopped at once.
        if over
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready()
        {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            failure.fail(NOT_STARTED);
            return;
        };
        // A window that was not prepared has no picture that a set could be held to, and gets no
        // key frame through. The app prepares every window that it starts.
        let size = prepared.unwrap_or_default();
        let link = Arc::new(SourceLink::new(handle, failure.clone(), size));
        let number = shared.links.add_source(Arc::clone(&link));
        if let Err(why) = window.start(number) {
            shared.links.remove(number);
            failure.fail(why.0.unwrap_or_else(|| NOT_STARTED.to_owned()));
            return;
        }
        // The task holds the link table and not the link: the table's entry is the only copy of
        // the handle that lasts, and it goes when the session is over.
        let links = Arc::clone(&shared.links);
        runtime.spawn(async move {
            over.await;
            links.remove(number);
            window.ended();
            // Released here, when the window drops.
        });
    }
}

/// What the viewer of the other device takes, for the platform to choose within.
fn what_it_takes(takes: &Takes) -> WhatItTakes {
    WhatItTakes {
        max_width: takes.max_width,
        max_height: takes.max_height,
        max_fps: u32::from(takes.max_fps),
        codecs: codec_bits(&takes.codecs),
    }
}

/// The `CODEC_` bits for these codecs. A codec that this version does not know has none.
fn codec_bits(codecs: &[Codec]) -> u32 {
    codecs.iter().fold(0, |bits, codec| match codec {
        Codec::H264 => bits | CODEC_H264,
        Codec::Other => bits,
    })
}

/// The picture that the platform chose, as the session's format, if it is one the protocol
/// allows: the codec is H.264, each edge is from 1 to the largest picture, and the rate is from
/// 1 to the highest.
///
/// # Errors
///
/// [`Error::Argument`] if it is not.
pub(crate) fn format_of(chosen: &Chosen) -> Result<Format, Error> {
    let fps = u16::try_from(chosen.fps).map_err(|_| Error::Argument)?;
    let edges = 1..=MAX_PICTURE_EDGE;
    if chosen.codec != CODEC_H264
        || !edges.contains(&chosen.width)
        || !edges.contains(&chosen.height)
        || !(1..=MAX_FRAME_RATE).contains(&fps)
    {
        return Err(Error::Argument);
    }
    Ok(Format {
        epoch: 0,
        codec: Codec::H264,
        width: chosen.width,
        height: chosen.height,
        fps,
    })
}

/// A picture as the platform gave it, read and bounded.
pub(crate) struct Made<'a> {
    /// Whether a later frame may refer to it.
    pub(crate) referenced: bool,
    /// When it was captured, on the clock of the session.
    pub(crate) capture_time: u64,
    /// The bytes of each length in the sample: 1, 2 or 4.
    pub(crate) length_size: usize,
    /// The picture in AVCC form.
    pub(crate) sample: &'a [u8],
    /// The parameter sets of the encoder's format, each without a start code.
    pub(crate) sets: Vec<&'a [u8]>,
}

/// What became of a picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// It went out as the frame with this number.
    Sent(u64),
    /// It is of no use any more.
    Stale,
    /// It is larger than the protocol allows.
    TooLarge,
    /// The session is over.
    Over,
}

impl Outcome {
    /// The `SENT_` number the platform is given.
    pub(crate) const fn code(self) -> u32 {
        match self {
            Self::Sent(_) => SENT,
            Self::Stale => SENT_STALE,
            Self::TooLarge => SENT_TOO_LARGE,
            Self::Over => SENT_OVER,
        }
    }

    /// The number of the frame, 0 if none went out.
    pub(crate) const fn number(self) -> u64 {
        match self {
            Self::Sent(number) => number,
            _ => 0,
        }
    }
}

/// The session of a window that this device shares, as the platform reaches it with its link.
pub(crate) struct SourceLink {
    handle: Arc<SourceHandle>,
    failure: SharingFailure,
    /// The kind of the frame that `begin` announced and that nothing has answered yet.
    announced: Mutex<Option<FrameKind>>,
    /// The width and height of the picture that the other device was told to expect: the one that
    /// `prepare` chose, and then the one of each `new_epoch`. The sequence parameter sets of a key
    /// frame are held to it.
    size: Mutex<(u32, u32)>,
}

impl SourceLink {
    fn new(handle: Arc<SourceHandle>, failure: SharingFailure, size: (u32, u32)) -> Self {
        Self {
            handle,
            failure,
            announced: Mutex::new(None),
            size: Mutex::new(size),
        }
    }

    /// The microseconds since the session began: the clock of the capture times.
    pub(crate) fn clock(&self) -> u64 {
        u64::try_from(self.handle.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    /// Asks whether to make a picture, and which kind: a `BEGIN_` number.
    pub(crate) fn begin(&self) -> u32 {
        let mut announced = lock(&self.announced);
        let Some(kind) = self.handle.begin() else {
            return BEGIN_NOT_NOW;
        };
        *announced = Some(kind);
        match kind {
            FrameKind::Key => BEGIN_KEY,
            FrameKind::Recovery => BEGIN_RECOVERY,
            FrameKind::Delta => BEGIN_DELTA,
        }
    }

    /// The picture that `begin` announced could not be made.
    pub(crate) fn cancel(&self) {
        lock(&self.announced).take();
        self.handle.cancel();
    }

    /// Whether the next picture has to be something other than a delta.
    pub(crate) fn recovery_pending(&self) -> bool {
        self.handle.recovery_pending()
    }

    /// Sends the picture that `begin` announced.
    ///
    /// # Errors
    ///
    /// [`Error::Argument`] if no picture was announced, or this one cannot go on the wire: a
    /// length that reaches beyond its end, a unit that could be taken for more than one, parameter
    /// sets that are not, a key frame that lacks a sequence or a picture parameter set or holds no
    /// slice of an IDR picture, or one whose sequence parameter set describes a picture larger
    /// than the one the other device was told to expect. Nothing was sent, and the caller gives up
    /// on the frame that `begin` announced with [`cancel`](Self::cancel).
    pub(crate) fn sent(&self, picture: &Made<'_>) -> Result<Outcome, Error> {
        let Some(announced) = lock(&self.announced).take() else {
            return Err(Error::Argument);
        };
        let mut bytes = Vec::with_capacity(picture.sample.len().saturating_add(64));
        let Ok(summary) = h264::avcc_to_annex_b(
            picture.sample,
            picture.length_size,
            &picture.sets,
            &mut bytes,
        ) else {
            return Err(Error::Argument);
        };
        // A key frame that holds no IDR picture is one that no viewer could start from.
        if announced == FrameKind::Key && !summary.idr {
            return Err(Error::Argument);
        }
        // The sets go out with a picture that holds an IDR slice, and a decoder allocates by them.
        // The other device refuses a key frame whose set describes more than it takes, and
        // nothing on this one would say why: an encoder that codes at the size of the window and
        // not at the size that was announced, or one that was made before the last `new_epoch`,
        // would show nothing there and no failure here.
        if summary.idr && !self.fit(&picture.sets) {
            return Err(Error::Argument);
        }
        let sent = self.handle.sent(Picture {
            bytes,
            referenced: picture.referenced,
            ltr: None,
            capture_time: picture.capture_time,
        });
        Ok(match sent {
            Ok(Some(number)) => Outcome::Sent(number),
            Ok(None) => Outcome::Stale,
            Err(NotSent::TooLarge) => Outcome::TooLarge,
            Err(NotSent::Over) => Outcome::Over,
            // A reason that a newer version of the library has: the frame is given up on.
            Err(_) => {
                self.handle.cancel();
                Outcome::Stale
            }
        })
    }

    /// Whether every sequence parameter set among `sets` describes pictures no larger than the
    /// ones that the other device was told to expect. The picture parameter sets say nothing of
    /// the size.
    fn fit(&self, sets: &[&[u8]]) -> bool {
        let (width, height) = *lock(&self.size);
        sets.iter()
            .filter(|set| set.first().is_some_and(|byte| byte & 0x1f == h264::NAL_SPS))
            .all(|set| h264::picture_fits(set, width, height))
    }

    /// A picture of more than [`MAX_PICTURE_BYTES`](baton_proto::limits::MAX_PICTURE_BYTES) is not
    /// read. The frame that `begin` announced is given up on, and the platform is told that it
    /// was too large.
    pub(crate) fn too_large(&self) -> Outcome {
        self.cancel();
        Outcome::TooLarge
    }

    /// The size of the picture changed: starts a new epoch.
    ///
    /// # Errors
    ///
    /// [`Error::Argument`] if the format is beyond what the protocol allows or the viewer takes.
    pub(crate) fn new_epoch(&self, chosen: &Chosen) -> Result<(), Error> {
        let format = format_of(chosen)?;
        let mut size = lock(&self.size);
        self.handle.new_epoch(format).map_err(|_| Error::Argument)?;
        *size = (format.width, format.height);
        Ok(())
    }

    /// Stops the pictures for a while, for a `PAUSE_` reason.
    ///
    /// # Errors
    ///
    /// [`Error::Argument`] if the reason is not one that is known.
    pub(crate) fn pause(&self, reason: u32) -> Result<(), Error> {
        let reason = match reason {
            PAUSE_LOCKED => PauseReason::Locked,
            PAUSE_HIDDEN => PauseReason::Hidden,
            PAUSE_OTHER => PauseReason::Other,
            _ => return Err(Error::Argument),
        };
        self.handle.pause(reason);
        Ok(())
    }

    /// The pictures go on.
    pub(crate) fn resume(&self) {
        self.handle.resume();
    }

    /// Ends the session. With a sentence, something went wrong and the person is told what; with
    /// none, the person ended it and nothing is shown.
    pub(crate) fn end(&self, sentence: Option<String>) {
        match sentence {
            Some(sentence) => self.failure.fail(sentence),
            None => self.handle.end(),
        }
    }
}

#[cfg(test)]
#[allow(unsafe_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::{
        ptr,
        sync::Mutex as StdMutex,
        task::{Poll, Waker},
    };

    use super::*;
    use crate::{
        Registry,
        abi::{BatonBytes, BatonPicked, OK, PICKED_DISPLAY, PICKED_WINDOW, size_of_u32},
        fake_sharer::{FakeSharer, PickPlan, SharerCall},
    };

    type Pick = BoxFuture<'static, Result<Option<Box<dyn PickedWindow>>, PlatformError>>;

    fn chosen(codec: u32, width: u32, height: u32, fps: u32) -> Chosen {
        Chosen {
            codec,
            width,
            height,
            fps,
        }
    }

    /// A registry with the sharer of a platform of its own.
    fn sharing() -> (Arc<Registry>, Arc<FakeSharer>, Arc<dyn WindowSharer>) {
        let registry = Arc::new(Registry::new());
        let platform = FakeSharer::new();
        platform.register_in(&registry).unwrap();
        let sharer = registry.sharer().unwrap();
        (registry, platform, sharer)
    }

    fn poll(pick: &mut Pick) -> Poll<Result<Option<Box<dyn PickedWindow>>, PlatformError>> {
        pick.as_mut().poll(&mut Context::from_waker(Waker::noop()))
    }

    /// Polls a pick that the platform has not answered: the request is made to the platform.
    fn asked(pick: &mut Pick) {
        assert!(poll(pick).is_pending());
    }

    fn sentence(result: Poll<Result<Option<Box<dyn PickedWindow>>, PlatformError>>) -> String {
        match result {
            Poll::Ready(Err(error)) => error.message().to_owned(),
            Poll::Ready(Ok(picked)) => panic!("a pick that worked: {}", picked.is_some()),
            Poll::Pending => panic!("a pick that waits"),
        }
    }

    #[test]
    fn a_format_that_the_protocol_allows_is_taken_as_it_is() {
        for (width, height, fps) in [(1, 1, 1), (64, 48, 30), (3840, 2160, 60), (3840, 3840, 120)] {
            let format = format_of(&chosen(CODEC_H264, width, height, fps)).unwrap();
            assert_eq!(
                (format.epoch, format.codec, format.width, format.height),
                (0, Codec::H264, width, height)
            );
            assert_eq!(u32::from(format.fps), fps);
        }
    }

    #[test]
    fn a_format_that_the_protocol_does_not_allow_is_refused() {
        let right = chosen(CODEC_H264, 1280, 720, 30);
        assert!(format_of(&right).is_ok());
        let wrong = [
            ("no codec", Chosen { codec: 0, ..right }),
            ("two codecs", Chosen { codec: 3, ..right }),
            ("a codec that is not known", Chosen { codec: 2, ..right }),
            ("no width", Chosen { width: 0, ..right }),
            (
                "a width beyond the largest picture",
                Chosen {
                    width: 3841,
                    ..right
                },
            ),
            ("no height", Chosen { height: 0, ..right }),
            (
                "a height beyond the largest picture",
                Chosen {
                    height: 3841,
                    ..right
                },
            ),
            ("no rate", Chosen { fps: 0, ..right }),
            ("a rate beyond the highest", Chosen { fps: 121, ..right }),
            // A cut to 16 bits would make this a rate of 30.
            (
                "a rate beyond 16 bits",
                Chosen {
                    fps: 65_536 + 30,
                    ..right
                },
            ),
            (
                "the largest number",
                Chosen {
                    fps: u32::MAX,
                    ..right
                },
            ),
        ];
        for (what, format) in wrong {
            assert!(format_of(&format) == Err(Error::Argument), "{what}");
        }
    }

    #[test]
    fn the_codecs_that_the_other_device_takes_are_told_to_the_platform_as_bits() {
        assert_eq!(codec_bits(&[]), 0);
        assert_eq!(codec_bits(&[Codec::H264]), CODEC_H264);
        assert_eq!(codec_bits(&[Codec::Other]), 0);
        assert_eq!(
            codec_bits(&[Codec::Other, Codec::H264, Codec::H264]),
            CODEC_H264
        );
    }

    #[test]
    fn what_became_of_a_picture_is_told_with_a_number_and_a_frame() {
        let told = |outcome: Outcome| (outcome.code(), outcome.number());
        assert_eq!(told(Outcome::Sent(7)), (SENT, 7));
        // Frame 0 is a frame like the others: the code says that it went out.
        assert_eq!(told(Outcome::Sent(0)), (SENT, 0));
        assert_eq!(told(Outcome::Stale), (SENT_STALE, 0));
        assert_eq!(told(Outcome::TooLarge), (SENT_TOO_LARGE, 0));
        assert_eq!(told(Outcome::Over), (SENT_OVER, 0));
    }

    #[test]
    fn a_pick_is_made_to_the_platform_and_answered_with_the_window_the_person_chose() {
        let (registry, platform, sharer) = sharing();
        let mut pick = sharer.pick();
        // Nothing is asked of the platform before the future is polled.
        assert_eq!(platform.log(), []);
        asked(&mut pick);
        assert_eq!(platform.calls(0), [SharerCall::Pick { request: 1 }]);
        assert_eq!((registry.waiting_picks(), platform.waiting()), (1, vec![1]));

        platform
            .choose(&registry, 1, b"Report.pdf", b"Preview", (1440, 900))
            .unwrap();
        let Poll::Ready(Ok(Some(picked))) = poll(&mut pick) else {
            panic!("the pick was not answered with a window");
        };
        assert_eq!(picked.title(), "Report.pdf");
        assert_eq!(picked.app(), "Preview");
        assert_eq!(picked.size(), (1440, 900));
        assert!(!picked.display(), "a window was taken for a display");
        assert_eq!(registry.waiting_picks(), 0);
        assert_eq!((platform.windows_made(), platform.live_windows()), (1, 1));
        // The window is the Rust side's now, and goes back to the platform when it is dropped,
        // also when nothing was done with it.
        assert_eq!(platform.calls(1), []);
        drop(picked);
        assert_eq!(platform.calls(1), [SharerCall::Release]);
        assert_eq!(platform.live_windows(), 0);
        assert!(
            platform.complaints().is_empty(),
            "{:?}",
            platform.complaints()
        );
    }

    #[test]
    fn a_display_that_the_person_chose_is_a_display_with_the_size_it_has() {
        let (registry, platform, sharer) = sharing();
        let mut pick = sharer.pick();
        asked(&mut pick);
        platform
            .choose_display(&registry, 1, b"", (3024, 1964))
            .unwrap();
        let Poll::Ready(Ok(Some(picked))) = poll(&mut pick) else {
            panic!("the pick was not answered with a display");
        };
        assert!(picked.display());
        assert_eq!(
            (picked.title(), picked.app()),
            (String::new(), String::new())
        );
        assert_eq!(picked.size(), (3024, 1964));
        // A display is let go of as a window is.
        drop(picked);
        assert_eq!(platform.calls(1), [SharerCall::Release]);
        assert_eq!(platform.live_windows(), 0);
        assert!(
            platform.complaints().is_empty(),
            "{:?}",
            platform.complaints()
        );
    }

    #[test]
    fn a_person_who_gives_up_leaves_nothing_behind() {
        let (registry, platform, sharer) = sharing();
        let mut pick = sharer.pick();
        asked(&mut pick);
        platform.give_up(&registry, 1).unwrap();
        assert!(matches!(poll(&mut pick), Poll::Ready(Ok(None))));
        assert_eq!(registry.waiting_picks(), 0);
        // An empty sentence is no sentence: the person gave up.
        let mut pick = sharer.pick();
        asked(&mut pick);
        platform.fail_pick(&registry, 2, b"  \0  ").unwrap();
        assert!(matches!(poll(&mut pick), Poll::Ready(Ok(None))));
        assert_eq!(platform.windows_made(), 0);
        // And the platform was not asked to take the picker down, since it is gone.
        assert_eq!(
            platform.count(|call| matches!(call, SharerCall::CancelPick { .. })),
            0
        );
    }

    #[test]
    fn a_picker_that_fails_later_says_why_to_the_person() {
        let (registry, platform, sharer) = sharing();
        let long = vec![b'a'; 1000];
        let cases: [(&[u8], String); 4] = [
            (
                b"The list of windows is not available.",
                "The list of windows is not available.".to_owned(),
            ),
            // White space around it and the end of a C string are not the person's business.
            (
                b"  Screen recording is off.\0garbage",
                "Screen recording is off.".to_owned(),
            ),
            (b"Not\xff UTF-8", "Not\u{fffd} UTF-8".to_owned()),
            (&long, "a".repeat(256)),
        ];
        for (request, (text, expected)) in (1..).zip(cases) {
            let mut pick = sharer.pick();
            asked(&mut pick);
            platform.fail_pick(&registry, request, text).unwrap();
            assert_eq!(sentence(poll(&mut pick)), expected);
        }
        assert_eq!((registry.waiting_picks(), platform.windows_made()), (0, 0));
    }

    #[test]
    fn a_picker_that_cannot_be_shown_fails_at_once_with_what_the_platform_said() {
        let (registry, platform, sharer) = sharing();
        let fails = |code: i32, text: &[u8], claimed_len: u32| PickPlan::Fail {
            code,
            text: text.to_vec(),
            claimed_len,
        };
        let long = vec![b'b'; 400];
        let cases = [
            (
                fails(-1, b"Screen recording is off.", 24),
                "Screen recording is off.",
            ),
            (fails(-1, b"", 0), NO_PICKER),
            (fails(7, b"   ", 3), NO_PICKER),
            // A length beyond the room is read as far as there is room.
            (fails(-1, &long, 400), &*"b".repeat(256)),
            // And one that is less than what was written is what counts.
            (fails(-1, b"Sorry about that.", 5), "Sorry"),
        ];
        for (plan, expected) in cases {
            platform.plan_pick(plan);
            let mut pick = sharer.pick();
            assert_eq!(sentence(poll(&mut pick)), expected);
            assert_eq!((registry.waiting_picks(), platform.waiting()), (0, vec![]));
        }
        // The picker was never up, so nothing was taken down, and nothing is waiting.
        assert_eq!(
            platform.count(|call| matches!(call, SharerCall::CancelPick { .. })),
            0
        );
        assert!(
            platform.complaints().is_empty(),
            "{:?}",
            platform.complaints()
        );
    }

    #[test]
    fn a_picker_that_is_up_is_up_whatever_the_platform_wrote_in_the_message() {
        let (registry, platform, sharer) = sharing();
        // The code says whether it worked, and a text beside a code that says yes is not read.
        platform.plan_pick(PickPlan::Fail {
            code: OK,
            text: b"Nothing is wrong.".to_vec(),
            claimed_len: 17,
        });
        let mut pick = sharer.pick();
        asked(&mut pick);
        assert_eq!((registry.waiting_picks(), platform.waiting()), (1, vec![1]));
        platform.give_up(&registry, 1).unwrap();
        assert!(matches!(poll(&mut pick), Poll::Ready(Ok(None))));
        assert!(
            platform.complaints().is_empty(),
            "{:?}",
            platform.complaints()
        );
    }

    #[test]
    fn a_pick_that_nobody_waits_for_any_more_is_taken_down_and_a_late_answer_is_refused() {
        let (registry, platform, sharer) = sharing();
        // A future that is dropped before it was polled asks for nothing.
        drop(sharer.pick());
        assert_eq!(platform.log(), []);

        let mut pick = sharer.pick();
        asked(&mut pick);
        drop(pick);
        assert_eq!(
            platform.log(),
            [
                (0, SharerCall::Pick { request: 1 }),
                (0, SharerCall::CancelPick { request: 1 })
            ]
        );
        assert_eq!((registry.waiting_picks(), platform.waiting()), (0, vec![]));

        // The platform answers anyway: it is told so, and the window stays with it.
        assert_eq!(
            platform.choose(&registry, 1, b"Late", b"", (10, 10)),
            Err(Error::UnknownLink)
        );
        assert_eq!(platform.windows_made(), 1);
        assert_eq!(platform.live_windows(), 0);
        assert_eq!(platform.calls(1), []);
        assert_eq!(platform.give_up(&registry, 1), Err(Error::UnknownLink));
        assert_eq!(
            platform.fail_pick(&registry, 1, b"No"),
            Err(Error::UnknownLink)
        );
        assert!(
            platform.complaints().is_empty(),
            "{:?}",
            platform.complaints()
        );
    }

    #[test]
    fn an_answer_is_taken_once_and_only_for_a_pick_that_was_made() {
        let (registry, platform, sharer) = sharing();
        let (mut first, mut second) = (sharer.pick(), sharer.pick());
        asked(&mut first);
        asked(&mut second);
        assert_eq!(
            platform.log(),
            [
                (0, SharerCall::Pick { request: 1 }),
                (0, SharerCall::Pick { request: 2 })
            ]
        );
        // Numbers that were never given, and 0, are not picks.
        for request in [0, 3, 99, u64::MAX] {
            assert_eq!(
                platform.give_up(&registry, request),
                Err(Error::UnknownLink)
            );
        }
        assert_eq!(registry.waiting_picks(), 2);

        // The answers can come in any order, and each goes to its own pick.
        platform
            .choose(&registry, 2, b"Second", b"App", (20, 20))
            .unwrap();
        assert_eq!(platform.give_up(&registry, 2), Err(Error::UnknownLink));
        assert!(poll(&mut first).is_pending());
        platform.give_up(&registry, 1).unwrap();
        assert!(matches!(poll(&mut first), Poll::Ready(Ok(None))));
        let Poll::Ready(Ok(Some(picked))) = poll(&mut second) else {
            panic!("the second pick was not answered with a window");
        };
        assert_eq!(picked.title(), "Second");

        // A number is not used again, also after its pick was answered.
        let mut third = sharer.pick();
        asked(&mut third);
        assert_eq!(platform.last_request(), Some(3));
        drop(third);
        drop(picked);
        assert!(
            platform.complaints().is_empty(),
            "{:?}",
            platform.complaints()
        );
    }

    #[test]
    fn an_answer_that_cannot_be_read_still_ends_the_wait_and_the_window_stays_with_the_platform() {
        let (registry, platform, sharer) = sharing();
        let good = |window| BatonPicked {
            size: size_of_u32::<BatonPicked>(),
            width: 100,
            height: 50,
            kind: PICKED_WINDOW,
            context: window,
            title: BatonBytes::of(b"Title"),
            app: BatonBytes::of(b"App"),
        };
        let window = platform.new_window();
        let nowhere = BatonBytes {
            ptr: ptr::null(),
            len: 5,
        };
        let some_failure = BatonBytes::of(b"Both a window and a failure.");
        let cases = [
            (
                "a window with no pointer",
                BatonPicked {
                    context: ptr::null_mut(),
                    ..good(window)
                },
                None,
            ),
            (
                "no width",
                BatonPicked {
                    width: 0,
                    ..good(window)
                },
                None,
            ),
            (
                "no height",
                BatonPicked {
                    height: 0,
                    ..good(window)
                },
                None,
            ),
            (
                "a struct that is too short",
                BatonPicked {
                    size: size_of_u32::<BatonPicked>() - 1,
                    ..good(window)
                },
                None,
            ),
            (
                "a struct with no size",
                BatonPicked {
                    size: 0,
                    ..good(window)
                },
                None,
            ),
            (
                "a title with a length and no pointer",
                BatonPicked {
                    title: nowhere,
                    ..good(window)
                },
                None,
            ),
            (
                "a name with a length and no pointer",
                BatonPicked {
                    app: nowhere,
                    ..good(window)
                },
                None,
            ),
            ("a window and a failure", good(window), Some(some_failure)),
            (
                "a failure with a length and no pointer",
                good(window),
                Some(nowhere),
            ),
        ];
        for (request, (what, picked, failure)) in (1..).zip(cases) {
            let mut pick = sharer.pick();
            asked(&mut pick);
            assert_eq!(
                platform.answer_raw(&registry, request, Some(&picked), failure.as_ref()),
                Err(Error::Argument),
                "{what}"
            );
            // The person who waits is told, and the window was not taken.
            assert_eq!(sentence(poll(&mut pick)), NOT_CHOSEN, "{what}");
            assert_eq!(registry.waiting_picks(), 0, "{what}");
        }
        FakeSharer::keep(window);
        assert_eq!(platform.calls(1), [], "the platform's window was released");
        assert!(
            platform.complaints().is_empty(),
            "{:?}",
            platform.complaints()
        );
    }

    #[test]
    fn a_kind_of_pick_that_the_interface_does_not_have_is_an_answer_that_cannot_be_read() {
        let (registry, platform, sharer) = sharing();
        let window = platform.new_window();
        for (request, kind) in (1..).zip([PICKED_DISPLAY + 1, 7, u32::MAX]) {
            let picked = BatonPicked {
                size: size_of_u32::<BatonPicked>(),
                width: 100,
                height: 50,
                kind,
                context: window,
                title: BatonBytes::of(b"Title"),
                app: BatonBytes::of(b"App"),
            };
            let mut pick = sharer.pick();
            asked(&mut pick);
            assert_eq!(
                platform.answer_raw(&registry, request, Some(&picked), None),
                Err(Error::Argument),
                "kind {kind}"
            );
            // The person who waits is told, and the window was not taken.
            assert_eq!(sentence(poll(&mut pick)), NOT_CHOSEN, "kind {kind}");
            assert_eq!(registry.waiting_picks(), 0, "kind {kind}");
        }
        FakeSharer::keep(window);
        assert_eq!(platform.calls(1), [], "the platform's window was released");
        assert!(
            platform.complaints().is_empty(),
            "{:?}",
            platform.complaints()
        );
    }

    #[test]
    fn the_title_the_application_and_the_size_are_made_fit_to_show() {
        // What the platform gave (title, application, size) and what the person is shown.
        type Case<'a> = (
            &'a [u8],
            &'a [u8],
            (u32, u32),
            (&'a str, &'a str, (u32, u32)),
        );
        let (registry, platform, sharer) = sharing();
        let boundary = [vec![b'a'; 511], "é".as_bytes().to_vec()].concat();
        let app_boundary = [vec![b'a'; 255], "é".as_bytes().to_vec()].concat();
        let cases: [Case<'_>; 7] = [
            (
                b"Plain",
                b"Plain app",
                (640, 480),
                ("Plain", "Plain app", (640, 480)),
            ),
            // A zero byte ends the text, as it does in a C string.
            (b"Re\0port", b"Pre\0view", (1, 1), ("Re", "Pre", (1, 1))),
            (
                b"A\xffB",
                b"\xc3",
                (1, 1),
                ("A\u{fffd}B", "\u{fffd}", (1, 1)),
            ),
            // The title is cut to 512 bytes and the name to 256, at the end of a character.
            (
                &vec![b'a'; 600],
                &vec![b'b'; 300],
                (1, 1),
                (&"a".repeat(512), &"b".repeat(256), (1, 1)),
            ),
            (&boundary, b"", (1, 1), (&"a".repeat(511), "", (1, 1))),
            (b"", &app_boundary, (1, 1), ("", &"a".repeat(255), (1, 1))),
            // A window larger than the largest that is told reads as the largest.
            (b"", b"", (20_000, u32::MAX), ("", "", (16_384, 16_384))),
        ];
        for (request, (title, app, size, (expected_title, expected_app, expected_size))) in
            (1..).zip(cases)
        {
            let mut pick = sharer.pick();
            asked(&mut pick);
            platform
                .choose(&registry, request, title, app, size)
                .unwrap();
            let Poll::Ready(Ok(Some(picked))) = poll(&mut pick) else {
                panic!("the pick was not answered with a window");
            };
            assert_eq!(picked.title(), expected_title);
            assert_eq!(picked.app(), expected_app);
            assert_eq!(picked.size(), expected_size);
        }
        assert_eq!(platform.live_windows(), 0, "every window was released");
        assert!(
            platform.complaints().is_empty(),
            "{:?}",
            platform.complaints()
        );
    }

    #[test]
    fn a_window_that_arrives_when_nobody_listens_any_more_is_released() {
        // The future was dropped just after the answer took the request, so the window cannot be
        // given to anyone: the platform was told that the Rust side took it, and it lets go.
        let registry = Registry::new();
        let platform = FakeSharer::new();
        let (reply, answer) = oneshot::channel();
        drop(answer);
        let request = registry.picks.add(Waiting {
            reply,
            shared: Arc::new(Shared {
                calls: platform.platform(),
                links: Arc::clone(&registry.links),
            }),
        });
        assert_eq!(registry.waiting_picks(), 1);
        platform
            .choose(&registry, request, b"Nobody", b"", (10, 10))
            .unwrap();
        assert_eq!(registry.waiting_picks(), 0);
        assert_eq!(platform.calls(1), [SharerCall::Release]);
        assert_eq!(platform.live_windows(), 0);
    }

    #[test]
    fn a_pick_that_nothing_can_answer_any_more_ends_with_a_sentence() {
        let (registry, _platform, sharer) = sharing();
        let mut pick = sharer.pick();
        asked(&mut pick);
        // Whoever held the answer went away without giving one.
        drop(registry.picks.take(1));
        assert_eq!(sentence(poll(&mut pick)), NO_ANSWER);
    }

    #[test]
    fn the_platform_cannot_answer_from_inside_the_call_that_asks_the_pick() {
        let registry = Arc::new(Registry::new());
        let platform = FakeSharer::new();
        platform.register_in(&registry).unwrap();
        let heard = Arc::new(StdMutex::new(Vec::new()));
        let (inside, there, mine) = (
            Arc::clone(&heard),
            Arc::clone(&platform),
            Arc::clone(&registry),
        );
        platform.on_call(move |_, call| {
            if let SharerCall::Pick { request } = call {
                inside.lock().unwrap().push(there.give_up(&mine, *request));
            }
        });
        let mut pick = registry.sharer().unwrap().pick();
        asked(&mut pick);
        assert_eq!(*heard.lock().unwrap(), [Err(Error::Reentrant)]);
        // The pick waits as before, and is answered from outside.
        assert_eq!(registry.waiting_picks(), 1);
        platform.give_up(&registry, 1).unwrap();
        assert!(matches!(poll(&mut pick), Poll::Ready(Ok(None))));
    }

    #[test]
    fn picks_are_numbered_from_one_and_a_number_is_never_used_twice() {
        let (registry, platform, sharer) = sharing();
        let mut seen = Vec::new();
        for _ in 0..5 {
            let mut pick = sharer.pick();
            asked(&mut pick);
            let request = platform.last_request().unwrap();
            seen.push(request);
            if request % 2 == 0 {
                platform.give_up(&registry, request).unwrap();
            } else {
                drop(pick);
            }
        }
        assert_eq!(seen, [1, 2, 3, 4, 5]);
        assert_eq!((registry.waiting_picks(), platform.waiting()), (0, vec![]));
    }
}
