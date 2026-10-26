// SPDX-License-Identifier: Apache-2.0

//! Sessions that the viewer starts (`docs/protocol.md` section 9, ADR 0010).
//!
//! A person who wants to look at the screen of one of their other devices can ask that device to
//! start the session, instead of going to it to share something. The device that is asked, the
//! source, does not ask a person: the person at the source allowed this device beforehand, once,
//! in the settings for it ([`StartPolicy`](crate::StartPolicy)), and nothing else makes the source
//! act. What the source does then is what it does for any session. It makes an offer, under the id
//! that the viewer chose, and the viewer, which waits for exactly that offer from exactly that
//! device, takes it without asking its own person, who asked a moment ago.
//!
//! At the source the asking is [`answer_start`](Streaming::answer_start): the checks come in the
//! order that the protocol gives, then the application is asked to get ready
//! ([`StartHandler`]), and a yes is the answer `Starting`. The application then makes the offer
//! with the [`StartSlot`] that it was given ([`Node::share_asked`](crate::Node::share_asked)). At
//! the viewer the asking is [`Node::start`](crate::Node::start): it takes the viewer's one place
//! ([`Streaming::expect_start`]), asks, and waits for the offer.
//!
//! An offer is taken without asking only if it answers a start that this device made, from the
//! device that it was made to, under the id that it was made with, while that start waits. Every
//! other offer goes to the person as before, and so does one that is a moment too late.

use std::{fmt, sync::Arc, time::Duration};

use baton_proto::{
    DeviceId,
    id::SessionId,
    message::DeclineReason,
    stream::{
        Declined, NotStarted, Offer, SourceKind, Start, Starting, StreamReply, Wanted, WhyNot,
    },
};
use iroh::endpoint::Connection;
use tokio::sync::oneshot;

use super::{Granted, Reservation, Streaming, Table, ViewHandle, Viewing};
use crate::{
    error::{Error, Result},
    transfer::incoming::Link,
    trust::PairedDevice,
    util::{BoxFuture, lock},
};

/// What the application does when a paired device that is allowed to asks this device to start a
/// session. A device that was given a handler advertises `stream.start`.
///
/// By the time [`start`](Self::start) is called, the request is well-formed, the device is paired,
/// the settings for it allow it to start a session, it has not asked too often, and this device
/// is not offering or sharing anything else. The person at this device is not asked: they decided
/// when they allowed the device.
pub trait StartHandler: Send + Sync + 'static {
    /// Gets ready to share what the device wants to see, and says whether it will.
    ///
    /// The handler has [`Tuning::start_prepare`](crate::Tuning) to answer, and the device is told
    /// that the start failed when the time is up. The call is dropped then, and when the device
    /// goes away, and the slot in it is let go of. On [`Starting`](StartVerdict::Starting) the
    /// handler has the slot, or has handed it to whatever will make the offer with
    /// [`Node::share_asked`](crate::Node::share_asked); a slot that is dropped instead lets go of
    /// the place, and the device waits in vain until its own time is up.
    fn start<'a>(
        &'a self,
        from: &'a PairedDevice,
        request: StartRequest,
    ) -> BoxFuture<'a, StartVerdict>;
}

// An application that keeps a handle to its handler hands the node a clone of it.
impl<T: StartHandler + ?Sized> StartHandler for Arc<T> {
    fn start<'a>(
        &'a self,
        from: &'a PairedDevice,
        request: StartRequest,
    ) -> BoxFuture<'a, StartVerdict> {
        (**self).start(from, request)
    }
}

/// What a paired device asked this one to start.
#[derive(Debug)]
#[non_exhaustive]
pub struct StartRequest {
    /// What the device wants to see. It is never [`Wanted::Other`]: a request for something this
    /// version does not know is answered without asking the handler.
    pub wanted: Wanted,
    /// Whether the device may control what it sees: it asked to, and the settings for it allow it.
    /// An offer made with this slot grants no more control than this.
    pub control: bool,
    /// The place of the session, and the right to make the offer. Handed back with the window
    /// to [`Node::share_asked`](crate::Node::share_asked).
    pub slot: StartSlot,
}

/// What the handler says to a start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StartVerdict {
    /// This device will make the offer, with the slot of the request.
    Starting,
    /// It will not, for this reason. The device that asked is told, and the place is let go of.
    NotStarted(WhyNot),
}

/// The place of a session that a paired device asked this one to start, and the right to make the
/// offer for it. Only [`Node::share_asked`](crate::Node::share_asked) takes it, so an offer under
/// the id that the device chose can be made only for a device that was allowed to ask.
///
/// The place is let go of when the slot is dropped.
#[must_use = "the place is let go of when the slot is dropped"]
pub struct StartSlot {
    pub(crate) reservation: Reservation,
    pub(crate) session: SessionId,
    pub(crate) peer: DeviceId,
    pub(crate) control: bool,
}

impl StartSlot {
    /// The device that asked.
    #[must_use]
    pub fn peer(&self) -> DeviceId {
        self.peer
    }

    /// Whether the offer may grant it control. The same as [`StartRequest::control`].
    #[must_use]
    pub fn control(&self) -> bool {
        self.control
    }
}

impl fmt::Debug for StartSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StartSlot")
            .field("peer", &self.peer.short())
            .field("control", &self.control)
            .finish_non_exhaustive()
    }
}

/// What came of a start that this device made.
#[derive(Debug)]
#[non_exhaustive]
pub enum StartOutcome {
    /// The other device made the offer, it was taken without asking a person, and the session
    /// runs. The handle controls it, and dropping it ends the session. The window handler is not
    /// told of this session: the caller has the handle.
    Started(ViewHandle),
    /// The other device did not start a session, or started one and gave up on it. The reason is
    /// what a person can be told.
    NotStarted(WhyNot),
}

/// What a device needs to start a session when a paired device asks it to.
pub(crate) struct Starter {
    handler: Arc<dyn StartHandler>,
    /// Whether the device may ask once more now. It counts the asking.
    charge: Box<dyn Fn(&DeviceId) -> bool + Send + Sync>,
    /// How long the handler has to get ready.
    prepare: Duration,
}

impl Starter {
    pub(crate) fn new(
        handler: Arc<dyn StartHandler>,
        charge: impl Fn(&DeviceId) -> bool + Send + Sync + 'static,
        prepare: Duration,
    ) -> Self {
        Self {
            handler,
            charge: Box::new(charge),
            prepare,
        }
    }
}

/// The one start that a viewer made and the source has not offered for yet.
pub(super) struct Pending {
    /// The source it was made to.
    pub(super) peer: DeviceId,
    pub(super) session: SessionId,
    /// What kind of source the offer has to be.
    source: SourceKind,
    /// Whether the person wished to control what they see. The offer grants no more than this.
    control: bool,
    /// What the viewer takes, and the decoder that shows it, for the session that begins.
    viewing: Viewing,
    /// Gives the session to the call that started it, or says why there is none.
    pub(super) done: oneshot::Sender<std::result::Result<ViewHandle, WhyNot>>,
}

/// The place of a start in the table, while it waits for the offer. It is let go of when this is
/// dropped, whatever ended the wait, including the call being dropped.
#[must_use = "the place is let go of when this is dropped"]
pub(crate) struct Expecting {
    streaming: Arc<Streaming>,
    session: SessionId,
    /// The session, when the offer has come and was taken.
    pub(crate) offer: oneshot::Receiver<std::result::Result<ViewHandle, WhyNot>>,
}

impl Drop for Expecting {
    fn drop(&mut self) {
        lock(&self.streaming.table)
            .starting
            .take_if(|pending| pending.session == self.session);
    }
}

impl Table {
    /// Whether `peer` has a start going under `session` here, as a source: the place is taken for
    /// it, or the session it led to runs.
    fn started_by(&self, peer: &DeviceId, session: SessionId) -> bool {
        self.sending
            .as_ref()
            .is_some_and(|sending| sending.peer == *peer && sending.asked == Some(session))
            || self
                .sessions
                .get(&session)
                .is_some_and(|running| running.is_source() && running.peer() == *peer)
    }
}

impl Streaming {
    /// Whether this device starts a session when a paired device asks it to, which is whether it
    /// advertises `stream.start`.
    pub(crate) fn starts(&self) -> bool {
        self.sharer
            .as_ref()
            .is_some_and(|sharer| sharer.starter.is_some())
    }

    /// Answers a start that `paired` made on `connection`, which has the link `link`.
    ///
    /// The checks come in the order of section 9: whether this device takes part at all and starts
    /// sessions, whether it knows what is wanted, whether the settings for the device allow it,
    /// whether the id was seen on this connection, whether this is a repeat of a start that is
    /// going, whether the device asked too often, and whether this device is busy; then the
    /// application, which has [`Tuning::start_prepare`](crate::Tuning).
    pub(super) async fn answer_start(
        self: &Arc<Self>,
        paired: &PairedDevice,
        link: Link,
        start: Start,
    ) -> StreamReply {
        let session = start.session;
        let not_started = |why| StreamReply::NotStarted(NotStarted { session, why });
        // A device that takes no part in window streaming answers every request as unsupported.
        if !self.takes_part() {
            return StreamReply::Declined(Declined {
                session,
                reason: DeclineReason::Unsupported,
            });
        }
        let Some(starter) = self
            .sharer
            .as_ref()
            .and_then(|sharer| sharer.starter.as_ref())
        else {
            return not_started(WhyNot::Unsupported);
        };
        if start.wanted == Wanted::Other {
            return not_started(WhyNot::Unsupported);
        }
        // The one thing that makes this device act: the person here allowed the device.
        let allowed = paired.policy.start;
        if !allowed.allows_start() {
            return not_started(WhyNot::NotAllowed);
        }
        // Control is granted only if the viewer wished for it, the person here allowed it, and this
        // device takes input at all: a source that does not must not set `control` (section 9),
        // and an application that offered it would fail to make the offer.
        let control = start.control && allowed.allows_control() && self.takes_control();

        let slot = {
            let mut table = lock(&self.table);
            // An id that was seen on this connection is refused, as for an offer: nothing is
            // repeated under a session id on a connection.
            if !table.offers.note(link, session) {
                return not_started(WhyNot::Invalid);
            }
            // The same start on another connection is a retry, because the first answer was lost
            // with its connection. It gets the answer the first got, and is not charged.
            if table.started_by(&paired.id, session) {
                return StreamReply::Starting(Starting { session });
            }
            if !(starter.charge)(&paired.id) {
                return not_started(WhyNot::RateLimited);
            }
            if table.sessions.contains_key(&session) {
                return not_started(WhyNot::Invalid);
            }
            match self.reserve_in(&mut table, paired.id, Some(session)) {
                Ok(reservation) => StartSlot {
                    reservation,
                    session,
                    peer: paired.id,
                    control,
                },
                Err(_busy) => return not_started(WhyNot::Busy),
            }
        };

        tracing::info!(
            peer = %paired.id.short(),
            control,
            "a paired device that is allowed to asked this device to start a session"
        );
        let taken_back = slot.reservation.cancellation();
        let request = StartRequest {
            wanted: start.wanted,
            control,
            slot,
        };
        // The slot is in the request, which is in the call. When the time is up, when the device
        // takes its asking back (or is unpaired, or this node stops), or when it goes away and
        // this answer is dropped, the call is dropped, and the place is let go of. An application
        // that is still getting ready is not left to do it for nobody, and the device can ask
        // again at once. The device that took its asking back does not read the answer.
        let prepared =
            tokio::time::timeout(starter.prepare, starter.handler.start(paired, request));
        tokio::select! {
            biased;
            () = taken_back.cancelled() => not_started(WhyNot::Failed),
            verdict = prepared => match verdict {
                Ok(StartVerdict::Starting) => StreamReply::Starting(Starting { session }),
                Ok(StartVerdict::NotStarted(why)) => not_started(why),
                Err(_elapsed) => not_started(WhyNot::Failed),
            },
        }
    }

    /// Takes the place of the one window that a viewer shows, for the offer that a start of this
    /// device will bring. The call that started it waits on `offer` of what this returns.
    ///
    /// # Errors
    ///
    /// This device was not set up to show the windows of others, or what is wanted is not
    /// something this version can ask for ([`Error::Config`], [`Error::BadItem`]); or a window is
    /// shown, or an offer waits for the person, or another start waits ([`Error::Busy`]).
    pub(crate) fn expect_start(
        self: &Arc<Self>,
        peer: DeviceId,
        session: SessionId,
        wanted: Wanted,
        control: bool,
        viewing: Viewing,
    ) -> Result<Expecting> {
        if !self.views() {
            return Err(Error::Config(
                "this device was not set up to show the windows of other devices".to_owned(),
            ));
        }
        let source = match wanted {
            Wanted::Screen => SourceKind::Display,
            Wanted::Other => {
                return Err(Error::BadItem(
                    "what is wanted is not something this version can ask for".to_owned(),
                ));
            }
        };
        let (done, offer) = oneshot::channel();
        let mut table = lock(&self.table);
        if table.offers.waiting.is_some() || table.has_view() || table.starting.is_some() {
            return Err(Error::Busy);
        }
        table.starting = Some(Pending {
            peer,
            session,
            source,
            control,
            viewing,
            done,
        });
        drop(table);
        Ok(Expecting {
            streaming: Arc::clone(self),
            session,
            offer,
        })
    }

    /// Takes the offer that answers the start `pending`, which `paired` was made, and starts the
    /// session, without asking a person. The caller holds the lock of the table, and has taken
    /// `pending` out of it, so no other offer can take the place meanwhile.
    ///
    /// The offer has to be of the kind that was asked for, and grants no more control than the
    /// person wished for. An offer of another kind is declined, and the call that started hears
    /// that the source did not do what it said.
    pub(super) fn take_asked(
        self: &Arc<Self>,
        table: &mut Table,
        paired: &PairedDevice,
        connection: &Connection,
        pending: Pending,
        offer: &Offer,
        sounding: bool,
    ) -> StreamReply {
        let session = offer.session;
        let declined = |reason| StreamReply::Declined(Declined { session, reason });
        let Pending {
            source,
            control,
            viewing,
            done,
            ..
        } = pending;
        if offer.source != source {
            let _ = done.send(Err(WhyNot::Failed));
            return declined(DeclineReason::Invalid);
        }
        let baton_proto::message::Response::Stream(reply) = viewing.accepted(session) else {
            let _ = done.send(Err(WhyNot::Failed));
            return declined(DeclineReason::Refused);
        };
        // The viewer takes no more control than it wished for.
        let granted = Granted {
            control: control && offer.control,
            private: offer.private,
            sound: sounding,
        };
        match self.view_in(table, paired, connection.clone(), session, viewing, granted) {
            Ok(handle) => {
                // A call that is dropped just now drops the handle in the channel with it, and
                // that ends the session. The place in the table is gone already: the caller took
                // it out under the lock, and the drop of the call waits for that lock.
                let _ = done.send(Ok(handle));
                reply
            }
            Err(error) => {
                tracing::warn!(%error, "a window could not be shown");
                let _ = done.send(Err(WhyNot::Failed));
                declined(DeclineReason::Invalid)
            }
        }
    }
}
