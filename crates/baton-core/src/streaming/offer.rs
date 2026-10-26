// SPDX-License-Identifier: Apache-2.0

//! What a node does to show one of its windows to a paired device (`docs/protocol.md`
//! section 9).
//!
//! The person at the source chose a window and a device, and [`Node::share`](crate::Node::share)
//! makes the offer. The viewer decides once, and a person may be asked, so the answer can take as
//! long as a person may take. An offer that is not answered is withdrawn with a `Stop`: when the
//! wait runs out, when the call fails, and when the call is dropped because the person changed
//! their mind. On a yes the application is shown what the viewer can take and chooses the picture
//! format within it, and the source's side of the session starts. The connection is held open for
//! as long as the session runs, since the pool sees requests only, and a window that is shown
//! makes none.
//!
//! A device has one session as a source at a time in this version, so it makes no offer while
//! another is waiting for an answer or running.
//!
//! The person at the source is not always the one who starts an offer. A paired device that the
//! person allowed beforehand may ask for one (`start.rs`), and the offer is then made for the
//! place that the asking took, under the id that the asking chose.
//!
//! A viewer has one place too, for an offer that waits for its person or a window that is shown,
//! and it lets go of it when it answers a `Stop` for it. So a `Stop` that this device sent (when
//! an offer was given up, when a session was ended here) is waited for, a few seconds at most,
//! before the next offer to the same device goes out: an offer that is made straight after would
//! otherwise meet the viewer's place still taken, and be refused as `RateLimited` by a device
//! that is not busy.

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use baton_proto::{
    DeviceId,
    id::SessionId,
    message::{DeclineReason, Message, Request, Response},
    stream::{Offer, SourceKind, Stop, StopReason, StreamReply, StreamRequest},
};
use baton_stream::control::Config;
use iroh::endpoint::Connection;
use tokio_util::sync::CancellationToken;

use super::{
    SoundPlan, SourceEvents, SourceHandle, SourceParams, SourceSetup, Streaming, Table, Takes,
    start::Starter,
};
use crate::{
    error::{Error, Result},
    session::{Hold, call},
    trust::PairedDevice,
    util::lock,
};

/// How long a `Stop` has to be answered. It is told to a device that may be gone.
const STOP_WAIT: Duration = Duration::from_secs(10);

/// How long an offer waits for the `Stop`s that went to the same device before it. A device that
/// is reachable answers in a moment; one that is not must not keep a person waiting, and its
/// answer to the offer, if the place is still taken, is `RateLimited`, as it was.
pub(crate) const SETTLE: Duration = Duration::from_secs(3);

/// What a device needs to take part as a source.
pub(crate) struct Sharer {
    /// Whether the device takes input from a viewer in a session where the person granted
    /// control, which is whether it advertises `stream.control`.
    pub(crate) control: bool,
    /// Whether the device keeps its screen private while a viewer controls it, and shows it to the
    /// people there when the viewer asks, which is whether it advertises `stream.private`. Only
    /// a device that takes input can.
    pub(crate) private: bool,
    /// What it needs to start a session when a paired device asks it to, if it does, which is
    /// whether it advertises `stream.start`.
    pub(crate) starter: Option<Starter>,
    /// Whether the device sends the sound of a display it shares, which is whether it advertises
    /// `stream.sound`.
    pub(crate) sound: bool,
}

/// The place of the one session that a device has as a source, from the offer to the session.
pub(super) struct Sending {
    /// The device the window is offered to.
    pub(super) peer: DeviceId,
    /// The session that the device asked for, if this place was taken because it asked, and none
    /// if the person here chose the window.
    pub(super) asked: Option<SessionId>,
    /// Cancelled when the device is unpaired or the node stops, or takes its asking back.
    pub(super) cancel: CancellationToken,
    /// Set before `cancel` is, when the device took its asking back.
    pub(super) withdrawn: Arc<AtomicBool>,
}

impl Sending {
    /// Cancels the offer because the device that asked for it took the asking back.
    pub(super) fn withdraw(&self) {
        self.withdrawn.store(true, Ordering::SeqCst);
        self.cancel.cancel();
    }
}

/// The place of the one session that a device has as a source. It is let go of when this is
/// dropped, which is when the offer has come to nothing or the session has been made.
#[must_use = "the place is let go of when the reservation is dropped"]
pub(crate) struct Reservation {
    streaming: Arc<Streaming>,
    cancel: CancellationToken,
    withdrawn: Arc<AtomicBool>,
}

impl Reservation {
    /// Whether the device was unpaired, or the node stopped, since the place was taken, or the
    /// device that asked for the offer took its asking back.
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Waits until the device is unpaired or the node stops, or the device that asked for the
    /// offer takes its asking back.
    pub(crate) async fn cancelled(&self) {
        self.cancel.cancelled().await;
    }

    /// The same cancellation, to wait on while the reservation is held by something else.
    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Whether the cancellation is the device taking its asking back, and not the device being
    /// unpaired or the node stopping.
    pub(crate) fn is_withdrawn(&self) -> bool {
        self.withdrawn.load(Ordering::SeqCst)
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        lock(&self.streaming.table).sending = None;
    }
}

/// A window, or a whole display, that the person at this device chose to show to another device.
///
/// The title and the name of the application are what the person at the other device is shown
/// before they decide, and they are never written to a log.
#[derive(Clone, PartialEq, Eq)]
pub struct WindowShare {
    source: SourceKind,
    title: String,
    app: String,
    width: u32,
    height: u32,
    control: bool,
    private: bool,
}

impl WindowShare {
    /// A window of `width` by `height` pixels with the title `title`, of the application `app`.
    /// Either name can be empty. The viewer may only look at the window.
    #[must_use]
    pub fn new(title: impl Into<String>, app: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            source: SourceKind::Window,
            title: title.into(),
            app: app.into(),
            width,
            height,
            control: false,
            private: false,
        }
    }

    /// What is shared is a whole display, with every window on it, and not one window: the size
    /// is the display's, and the title is its name, if the platform knows one, and can be empty.
    #[must_use]
    pub fn whole_display(mut self) -> Self {
        self.source = SourceKind::Display;
        self
    }

    /// The person at this device lets the viewer control the window. The viewer cannot ask for
    /// it and cannot turn it on later, so it is decided here. The node must have been set up to
    /// take input (`NodeBuilder::share_windows`). A whole display is offered with control as a
    /// window is, and the viewer then steers everything that is on it (protocol section 9).
    #[must_use]
    pub fn with_control(mut self) -> Self {
        self.control = true;
        self
    }

    /// The platform keeps the screen of this device private for as long as the session lasts: dark
    /// and out of the hands of the people here. The viewer is told, and can have the screen shown
    /// and the keys and the pointer put to use, and put either back, with
    /// [`ViewHandle::lift`](super::ViewHandle::lift). The node must have been set up to do that
    /// (`NodeBuilder::private_sessions`).
    #[must_use]
    pub fn with_private(mut self) -> Self {
        self.private = true;
        self
    }

    /// The offer that asks a device to show this window under `session`.
    pub(crate) fn offer(&self, session: SessionId) -> Offer {
        Offer {
            session,
            source: self.source,
            title: self.title.clone(),
            app: self.app.clone(),
            width: self.width,
            height: self.height,
            control: self.control,
            private: self.private,
        }
    }

    /// Whether the offer is within the limits of the protocol, and of what a source may offer.
    pub(crate) fn check(&self) -> Result<()> {
        Request::Stream(StreamRequest::Offer(
            self.offer(SessionId::from_bytes([0; SessionId::LEN])),
        ))
        .validate()
        .map_err(|error| Error::BadItem(error.to_string()))
    }

    /// Whether the viewer is to control the window.
    pub(crate) fn control(&self) -> bool {
        self.control
    }

    /// Whether what is shared is a whole display.
    pub(crate) fn is_display(&self) -> bool {
        self.source == SourceKind::Display
    }

    /// Whether the screen is kept private for the session.
    pub(crate) fn private(&self) -> bool {
        self.private
    }
}

// What a window is called can name a document, a person or a page.
impl fmt::Debug for WindowShare {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowShare")
            .field("source", &self.source)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("control", &self.control)
            .field("private", &self.private)
            .finish_non_exhaustive()
    }
}

/// What came of an offer of a window.
#[derive(Debug)]
#[non_exhaustive]
pub enum ShareOutcome {
    /// The person at the other device accepted, and the session runs. The handle controls it:
    /// the platform asks it whether to encode a picture and hands it what it encoded, and
    /// dropping it ends the session.
    Started(SourceHandle),
    /// The other device did not take the window, and says why. A `Refused` is the person saying
    /// no, or not answering in time.
    Declined(DeclineReason),
    /// The other device accepted, but the application found nothing in what it takes that it can
    /// make. The session was ended at once.
    Unmatched,
    /// The other device had asked for the offer and took its asking back before the offer was
    /// answered. Nothing is shared. Only an offer that a device asked for ends this way
    /// ([`Node::share_asked`](crate::Node::share_asked)).
    Cancelled,
}

/// An offer that was made and answered.
pub(crate) struct Offered<'a> {
    /// The device the window was offered to.
    pub(crate) peer: &'a PairedDevice,
    /// The connection the offer went on, and the session lives on.
    pub(crate) connection: &'a Connection,
    /// Keeps the connection in the pool for as long as the session runs.
    pub(crate) hold: Hold,
    /// The session the offer was made under.
    pub(crate) session: SessionId,
    /// Whether the viewer is to control the window.
    pub(crate) control: bool,
    /// Whether the screen is kept private for the session.
    pub(crate) private: bool,
    /// What the source does about sound in the session.
    pub(crate) sound: SoundPlan,
}

impl Streaming {
    /// Whether this device takes part as a source, which is whether it advertises `stream.share`.
    pub(crate) fn shares(&self) -> bool {
        self.sharer.is_some()
    }

    /// Whether this device sends the sound of a display it shares, which is whether it advertises
    /// `stream.sound`.
    pub(crate) fn sounds(&self) -> bool {
        self.sharer.as_ref().is_some_and(|sharer| sharer.sound)
    }

    /// Whether this device takes input from a viewer, which is whether it advertises
    /// `stream.control`.
    pub(crate) fn takes_control(&self) -> bool {
        self.sharer.as_ref().is_some_and(|sharer| sharer.control)
    }

    /// Whether this device keeps its screen private while a viewer controls it, which is whether
    /// it advertises `stream.private`.
    pub(crate) fn keeps_private(&self) -> bool {
        self.sharer.as_ref().is_some_and(|sharer| sharer.private)
    }

    /// Whether this device can offer `window`: it takes part as a source, takes input if the
    /// person granted control, and keeps the screen private if it is to be.
    ///
    /// # Errors
    ///
    /// The device was not set up to show its windows, or not to take input, or not to keep a
    /// screen private, or the screen is private and control is not granted, or the window is
    /// beyond a limit of the protocol.
    pub(crate) fn can_offer(&self, window: &WindowShare) -> Result<()> {
        if !self.shares() {
            return Err(Error::Config(
                "this device was not set up to show its windows to others".to_owned(),
            ));
        }
        if window.control() && !self.takes_control() {
            return Err(Error::Config(
                "control of a window was granted, but this device was not set up to take input"
                    .to_owned(),
            ));
        }
        if window.private() && !self.keeps_private() {
            return Err(Error::Config(
                "a private screen was offered, but this device was not set up to keep one"
                    .to_owned(),
            ));
        }
        // A private screen is for a person who controls it from the other device.
        if window.private() && !window.control() {
            return Err(Error::Config(
                "a private screen was offered without control, which it is kept private for"
                    .to_owned(),
            ));
        }
        window.check()
    }

    /// Takes the place of the one session that a source has, for an offer to `peer` that the
    /// person at this device chose.
    ///
    /// # Errors
    ///
    /// [`Error::Busy`]: another window is being offered or shared.
    pub(crate) fn reserve(self: &Arc<Self>, peer: DeviceId) -> Result<Reservation> {
        self.reserve_in(&mut lock(&self.table), peer, None)
    }

    /// The same, in the hold of the lock of the table that the caller has, so that a check of
    /// something else is not undone before the place is taken. `asked` is the session that `peer`
    /// asked for, if it did.
    pub(super) fn reserve_in(
        self: &Arc<Self>,
        table: &mut Table,
        peer: DeviceId,
        asked: Option<SessionId>,
    ) -> Result<Reservation> {
        if table.has_source() {
            return Err(Error::Busy);
        }
        let cancel = CancellationToken::new();
        let withdrawn = Arc::new(AtomicBool::new(false));
        table.sending = Some(Sending {
            peer,
            asked,
            cancel: cancel.clone(),
            withdrawn: Arc::clone(&withdrawn),
        });
        Ok(Reservation {
            streaming: Arc::clone(self),
            cancel,
            withdrawn,
        })
    }

    /// Starts the session that an answer to an offer begins, if it is a yes.
    ///
    /// `choose` is shown what the viewer takes and says what picture the platform makes within
    /// it, or that it can make none; a viewer that takes nothing the platform can make is told
    /// with a `Stop` whose reason is `Failed`, as section 9 asks.
    ///
    /// # Errors
    ///
    /// The answer was not one to an offer, or the session could not be made; the viewer is told
    /// with a `Stop` then too.
    pub(crate) async fn begin(
        self: &Arc<Self>,
        offered: Offered<'_>,
        reply: StreamReply,
        events: impl SourceEvents,
        choose: impl FnOnce(&Takes) -> Option<Config>,
    ) -> Result<ShareOutcome> {
        let accepted = match reply {
            StreamReply::Accepted(accepted) => accepted,
            StreamReply::Declined(declined) => return Ok(ShareOutcome::Declined(declined.reason)),
            StreamReply::Stopped(_) | StreamReply::Starting(_) | StreamReply::NotStarted(_) => {
                return Err(Error::protocol("the answer was not one to an offer"));
            }
        };
        let takes = Takes::of(&accepted);
        let Some(config) = choose(&takes) else {
            say_stop(offered.connection, offered.session, StopReason::Failed).await;
            return Ok(ShareOutcome::Unmatched);
        };
        let setup = SourceSetup {
            config,
            control: offered.control,
            private: offered.private,
            takes,
            sound: offered.sound,
            params: SourceParams::default(),
        };
        let started = self.source(
            offered.peer,
            offered.connection.clone(),
            Some(offered.hold),
            offered.session,
            setup,
            events,
        );
        match started {
            Ok(handle) => Ok(ShareOutcome::Started(handle)),
            Err(error) => {
                say_stop(offered.connection, offered.session, StopReason::Failed).await;
                Err(error)
            }
        }
    }
}

/// Asks the device at the other end of `connection` whether it will show a window, and waits as
/// long as `wait`, which is as long as a person may take and the network.
///
/// An offer that is not answered is withdrawn: this future is dropped when the person at this
/// device gives up, and then, as when the call fails or the wait runs out, the other device is
/// told with a `Stop`, so it takes the question back from its person. The answer is an
/// `Accepted` or a `Declined` for this session.
pub(crate) async fn ask(
    streaming: &Arc<Streaming>,
    peer: DeviceId,
    connection: &Connection,
    offer: Offer,
    wait: Duration,
) -> Result<StreamReply> {
    let session = offer.session;
    let mut withdraw = Withdraw::new(streaming, peer, connection, session);
    let request = Request::Stream(StreamRequest::Offer(offer));
    let reply = match call(connection, &request, wait).await? {
        Response::Stream(reply) if reply.session() == session => reply,
        _ => {
            return Err(Error::protocol(
                "the answer to an offer was not for that offer",
            ));
        }
    };
    if !matches!(reply, StreamReply::Accepted(_) | StreamReply::Declined(_)) {
        return Err(Error::protocol("the answer was not one to an offer"));
    }
    withdraw.disarm();
    Ok(reply)
}

/// Withdraws an offer that has not been answered, or a start whose offer has not come, when it
/// is dropped.
pub(crate) struct Withdraw {
    streaming: Arc<Streaming>,
    /// Sends the `Stop`: whichever thread drops the call, it need not be in the runtime.
    runtime: tokio::runtime::Handle,
    peer: DeviceId,
    connection: Connection,
    session: SessionId,
    armed: bool,
}

impl Withdraw {
    /// A guard that tells `peer`, on `connection`, that `session` is over, unless it is
    /// [`disarmed`](Self::disarm) first. Must be made in the runtime of the node.
    pub(crate) fn new(
        streaming: &Arc<Streaming>,
        peer: DeviceId,
        connection: &Connection,
        session: SessionId,
    ) -> Self {
        Self {
            streaming: Arc::clone(streaming),
            runtime: tokio::runtime::Handle::current(),
            peer,
            connection: connection.clone(),
            session,
            armed: true,
        }
    }

    /// There is nothing to withdraw any more: the other device answered, or ended it.
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for Withdraw {
    fn drop(&mut self) {
        if self.armed {
            self.streaming.stop_later(
                &self.runtime,
                self.peer,
                &self.connection,
                self.session,
                StopReason::Ended,
            );
        }
    }
}

/// Tells the device at the other end that the session, or the offer, is over.
///
/// Whether it is answered changes nothing for the session: a viewer learns of the end of a
/// session from its control stream too, and a question that it does not take back is dropped
/// with the connection.
async fn say_stop(connection: &Connection, session: SessionId, reason: StopReason) {
    let request = Request::Stream(StreamRequest::Stop(Stop { session, reason }));
    let _ = call(connection, &request, STOP_WAIT).await;
}

impl Streaming {
    /// Tells `peer` that the offer or the session `session` is over, from a place that cannot
    /// wait for the answer, such as a drop. The next offer to the same device waits for it
    /// ([`settled`](Self::settled)).
    ///
    /// The `Stop` is sent by a task of `runtime`. It is given and not looked for, since a drop
    /// runs on whichever thread lets go: the platform ends a session from threads of its own,
    /// which are not in the runtime, and from there no runtime would be found.
    pub(crate) fn stop_later(
        self: &Arc<Self>,
        runtime: &tokio::runtime::Handle,
        peer: DeviceId,
        connection: &Connection,
        session: SessionId,
        reason: StopReason,
    ) {
        let answered = CancellationToken::new();
        {
            let mut table = lock(&self.table);
            table.stops.retain(|(_, answered)| !answered.is_cancelled());
            table.stops.push((peer, answered.clone()));
        }
        // The guard cancels the token when the task is done, and when it is dropped without ever
        // having run, as it is when the runtime shuts down.
        let done = answered.drop_guard();
        let connection = connection.clone();
        runtime.spawn(async move {
            say_stop(&connection, session, reason).await;
            drop(done);
        });
    }

    /// Waits until the `Stop`s that were sent to `peer` have been answered, but no longer than
    /// `within`: a device that does not answer must not keep an offer back for long.
    pub(crate) async fn settled(&self, peer: &DeviceId, within: Duration) {
        let pending: Vec<CancellationToken> = lock(&self.table)
            .stops
            .iter()
            .filter(|(to, _)| to == peer)
            .map(|(_, answered)| answered.clone())
            .collect();
        let all_answered = async {
            for answered in pending {
                answered.cancelled().await;
            }
        };
        let _ = tokio::time::timeout(within, all_answered).await;
    }
}
