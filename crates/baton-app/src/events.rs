// SPDX-License-Identifier: Apache-2.0

//! What happens while the app runs, as it happens.

use tokio::sync::broadcast::{self, error::RecvError};
use tokio_util::sync::CancellationToken;

use crate::model::{
    OfferOutcome, PairingRequest, Received, TransferEnd, TransferRequest, WindowEnd,
    WindowNoticeReason, WindowRequest, WindowSession, WindowSound,
};

/// Something a screen may want to react to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A device handed this one something. It is in the inbox already.
    Received(Received),
    /// The list of paired devices changed: one was added, renamed, forgotten, or heard from.
    /// Read it again with [`App::devices`](crate::App::devices).
    DevicesChanged,
    /// A device wants to pair, using the offer that is showing. Show the request and answer
    /// it with [`App::answer_pairing`](crate::App::answer_pairing).
    PairingRequested(PairingRequest),
    /// A request no longer needs an answer: the offer ended, or the device gave up. Take the
    /// question off the screen.
    PairingRequestEnded {
        /// The [`PairingRequest::id`] that ended.
        id: u64,
    },
    /// The offer that was showing ended.
    OfferEnded(OfferOutcome),
    /// A device wants to send files. Show what it offers, with who is asking, and answer with
    /// [`App::answer_transfer`](crate::App::answer_transfer): the person has two minutes, and
    /// no answer is a no.
    TransferRequested(TransferRequest),
    /// A request for files no longer needs an answer: the other device gave up, was unpaired, or
    /// the time ran out. Take the question off the screen.
    TransferRequestEnded {
        /// The [`TransferRequest::id`] that ended.
        id: u64,
    },
    /// A transfer that the person said yes to has ended. The files that arrived are in the
    /// folder already.
    TransferEnded(TransferEnd),
    /// A paired device offers a window to show on this one. Show the request and answer it with
    /// [`App::answer_window`](crate::App::answer_window).
    WindowRequested(WindowRequest),
    /// A window request no longer needs an answer: the other device withdrew the offer, the time
    /// ran out, or the connection ended. Take the question off the screen.
    ///
    /// The person's own answer is not announced with this event: a screen takes the question
    /// down when the person answers, and draws the window when [`Event::WindowStarted`] says so.
    /// A yes that comes as the offer is withdrawn, or whose session then fails to begin, ends
    /// with no event at all.
    ///
    /// `failure` is the sentence to show when the person said yes and this device could not show
    /// the window after all. The other device was told that the window was refused.
    WindowRequestEnded {
        /// The [`WindowRequest::id`] that ended.
        id: u64,
        /// Why the window could not be shown, if the person accepted it.
        failure: Option<String>,
    },
    /// A window is shown on this device or shared from it. When it is shown here, the session
    /// names the screen to draw it with.
    WindowStarted(WindowSession),
    /// The picture of a window that is shown here is this many pixels wide and high. The source
    /// says so before its first picture and again whenever it changes the size, so a screen
    /// lays the picture out in this shape and not in that of the window that was offered: the
    /// source scales a window to a picture that fits what this device takes, and a window that
    /// is resized changes shape.
    ///
    /// It always comes after the [`Event::WindowStarted`] of the same window, and is not sent
    /// for a window that is shared from this device.
    WindowPicture {
        /// The [`WindowSession::id`] whose picture this is.
        id: u64,
        /// The width of the picture in pixels, at least 1.
        width: u32,
        /// The height of the picture in pixels, at least 1.
        height: u32,
    },
    /// The other device did not do some of what the person did on a window that is shown here:
    /// it dropped events, for the reason named, and `count` is how many. Say so in a sentence, so
    /// that a tap that did nothing is not a mystery.
    ///
    /// The other device sends at most one notice a second, naming the reason that cost the most
    /// events since the last, so a person who keeps tapping on a locked screen hears of it once a
    /// second. It always comes after the [`Event::WindowStarted`] of the same window, and is not
    /// sent for a window that is shared from this device.
    WindowNotice {
        /// The [`WindowSession::id`] whose events were dropped.
        id: u64,
        /// The rule that dropped the most of them.
        reason: WindowNoticeReason,
        /// How many events it dropped, at least 1.
        count: u32,
    },
    /// The sound of a screen that is shown here changed: it plays, the person turned it off, or the
    /// other device says why it sends none. Show the person how it stands.
    ///
    /// It comes only for a session whose [`WindowSession::sound`] is set, and only after the
    /// person asked to hear it with [`App::hear_window`](crate::App::hear_window), so it always
    /// comes after the [`Event::WindowStarted`] of the same window. The other device that said it
    /// sends none can begin by itself when the cause is gone, for example when the person there
    /// has allowed it to record the sound, and then [`WindowSound::Playing`] follows; a person
    /// who asks again with `hear_window` makes it try at once.
    WindowSound {
        /// The [`WindowSession::id`] whose sound this is.
        id: u64,
        /// How the sound stands now.
        sound: WindowSound,
    },
    /// A window that was shown or shared is over.
    WindowEnded {
        /// The [`WindowSession::id`] that ended.
        id: u64,
        /// How it ended.
        end: WindowEnd,
    },
    /// Events were missed because they came faster than they were read. Read the devices and
    /// the inbox again.
    ///
    /// A pairing request, a request for files, the end of an offer or the end of a transfer
    /// among the missed events is not recovered this way yet: a request nobody answers runs
    /// out as a no after two minutes, an offer ends when its time is up, and a transfer that
    /// was taken shows no end. The events of windows are not recovered either: a screen that
    /// missed `WindowStarted` cannot end that window short of stopping the app, and a request to
    /// show a window that it missed runs out as a no. A missed `WindowPicture` leaves the
    /// picture in the shape of the window that was offered until the next one comes, and a
    /// missed `WindowNotice` leaves the person without the reason, which the next one gives. A
    /// missed `WindowSound` leaves the sound as the screen last showed it; turning it off and on
    /// again asks again, and the answer tells how it stands.
    Resync,
}

/// A stream of [`Event`]s from one point on.
///
/// Get one with [`App::events`](crate::App::events) *before* reading the inbox or the
/// devices, so that whatever arrives while reading is not lost: it is either in what was
/// read or in the events that follow.
#[derive(Debug)]
pub struct Events {
    rx: broadcast::Receiver<Event>,
    stopped: CancellationToken,
}

impl Events {
    pub(crate) const fn new(rx: broadcast::Receiver<Event>, stopped: CancellationToken) -> Self {
        Self { rx, stopped }
    }

    /// Waits for the next event. Returns `None` once the app has stopped, and after that
    /// always.
    pub async fn next(&mut self) -> Option<Event> {
        tokio::select! {
            biased;
            () = self.stopped.cancelled() => None,
            received = self.rx.recv() => match received {
                Ok(event) => Some(event),
                Err(RecvError::Lagged(_)) => Some(Event::Resync),
                Err(RecvError::Closed) => None,
            },
        }
    }
}
