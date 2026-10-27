// SPDX-License-Identifier: Apache-2.0

//! What a viewer answers to the requests of a window streaming session: the offer of a window,
//! and the end of a session (`docs/protocol.md` section 9).
//!
//! A device that was given a [`WindowHandler`] shows the windows that paired devices offer. An
//! offer is checked in the order that the protocol gives: whether this device takes part at all,
//! whether the id of the session was seen on the connection before, whether another offer waits
//! for the person, a window is shown or the device sends too many items, and then the policy of
//! the application and the person, who has the time that a person has for an approval. A viewer
//! shows one window at a time.
//!
//! The question to the person is a future that the handler returns, and it is dropped when the
//! source withdraws the offer, when the connection ends, when the device is unpaired and when the
//! time is up, so the application takes the question back from the person then.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt,
    sync::Arc,
    time::Duration,
};

use baton_proto::{
    DeviceId,
    device::{MAX_DISPLAY_NAME_CHARS, MAX_DISPLAY_TITLE_CHARS, sanitize_text},
    id::SessionId,
    message::{DeclineReason, Hello, Message, Response, capability},
    stream::{
        Accepted, Codec, Declined, Offer, SourceKind, Stop, Stopped, StreamReply, StreamRequest,
        WhyNot,
    },
};
use iroh::endpoint::Connection;
use tokio_util::sync::CancellationToken;

use super::{
    Decoder, Granted, Speaker, Streaming, Takes, UnlockApprover, Unlocker, ViewHandle, ViewParams,
    approve::KeyPlace,
};
use crate::{
    error::{Error, Result},
    transfer::incoming::Link,
    trust::PairedDevice,
    util::{BoxFuture, lock},
};

/// How many ids of offers are remembered for each connection. An honest source makes one offer
/// for each window it shows, so this is far more than a connection sees.
const SEEN_PER_CONNECTION: usize = 256;

/// A window, or a whole display, that another device offers, as a person is told about it.
///
/// The title and the name of the application are text that the other device wrote. They are
/// cleaned here as a device name is (`sanitize_text`), so they are safe to show, but they are
/// still only labels: nothing in them tells a person which device they come from. They are never
/// written to a log.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WindowOffer {
    /// Whether what is offered is a whole display, with every window on it, and not one window.
    pub display: bool,
    /// The title of the window, or the name of the display, or `None` if it had none or nothing
    /// visible was left of it.
    pub title: Option<String>,
    /// The name of the application the window belongs to, or `None` as for the title.
    pub app: Option<String>,
    /// The width of the window in pixels.
    pub width: u32,
    /// The height of the window in pixels.
    pub height: u32,
    /// Whether the person at the other device granted this one control of the window. The viewer
    /// cannot ask for it and cannot turn it on later.
    pub control: bool,
    /// Whether the other device keeps its screen private for the session: dark, and out of the
    /// hands of the people there. This device can ask it to show the screen, and to let the people
    /// there use its keys and pointer, and to put either back ([`ViewHandle::lift`]).
    pub private: bool,
}

impl WindowOffer {
    fn new(offer: &Offer) -> Self {
        Self {
            display: offer.source == SourceKind::Display,
            title: sanitize_text(&offer.title, MAX_DISPLAY_TITLE_CHARS),
            app: sanitize_text(&offer.app, MAX_DISPLAY_NAME_CHARS),
            width: offer.width,
            height: offer.height,
            control: offer.control,
            private: offer.private,
        }
    }

    /// The offer that a device expects when it asked another for its whole screen
    /// ([`Node::start`](crate::Node::start)), for a platform that has to make the screen before
    /// the offer comes: a display with no name, and a size that is not known yet (nought by
    /// nought). `control` is whether the person wished to control it; the offer grants no more.
    #[must_use]
    pub fn asked_screen(control: bool) -> Self {
        Self {
            display: true,
            title: None,
            app: None,
            width: 0,
            height: 0,
            control,
            // Said by the offer when it comes, and then by `ViewHandle::private`.
            private: false,
        }
    }
}

// What a window is called can name a document, a person or a page.
impl fmt::Debug for WindowOffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowOffer")
            .field("display", &self.display)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("control", &self.control)
            .field("private", &self.private)
            .finish_non_exhaustive()
    }
}

/// What a viewer says when it takes a window: the largest picture it can show, the codecs it can
/// decode, and the decoder and screen that it shows the picture with, and, if it plays the sound
/// of a screen, the speaker.
pub struct Viewing {
    pub(super) decoder: Box<dyn Decoder>,
    pub(super) speaker: Option<Box<dyn Speaker>>,
    pub(super) unlocker: Option<Box<dyn Unlocker>>,
    pub(super) takes: Takes,
    // Boxed, so that the verdict that holds a `Viewing` is not too large for its other variants.
    pub(super) params: Box<ViewParams>,
}

impl Viewing {
    /// A viewer that shows pictures of up to `max_width` by `max_height` pixels at up to `max_fps`
    /// frames a second, in one of `codecs`, with `decoder`.
    ///
    /// # Errors
    ///
    /// A number is outside what the protocol allows ([`Error::Config`]): each edge from 1 to 3840
    /// pixels, the rate from 1 to 120, and from 1 to 8 codecs, none of them [`Codec::Other`].
    pub fn new(
        decoder: impl Decoder,
        max_width: u32,
        max_height: u32,
        max_fps: u16,
        codecs: Vec<Codec>,
    ) -> Result<Self> {
        if codecs.contains(&Codec::Other) {
            return Err(Error::Config(
                "what the viewer can show: a codec that is not known cannot be decoded".to_owned(),
            ));
        }
        let viewing = Self {
            decoder: Box::new(decoder),
            speaker: None,
            unlocker: None,
            takes: Takes {
                max_width,
                max_height,
                max_fps,
                codecs,
            },
            params: Box::default(),
        };
        viewing
            .accepted(SessionId::from_bytes([0; 16]))
            .validate()
            .map_err(|error| Error::Config(format!("what the viewer can show: {error}")))?;
        Ok(viewing)
    }

    /// Plays the sound of the screen that the session shows with `speaker`, when the application
    /// asks for it ([`ViewHandle::hear`]). Sound goes only with a whole display, and only to a
    /// device that was set up to play it
    /// ([`NodeBuilder::play_sound`](crate::NodeBuilder::play_sound)): on any other, the speaker is
    /// never used.
    #[must_use]
    pub fn with_speaker(mut self, speaker: impl Speaker) -> Self {
        self.speaker = Some(Box::new(speaker));
        self
    }

    /// Asks for the face of the person with `unlocker` when the source of the session asks a
    /// question to unlock its screen, which the person can start with
    /// [`ViewHandle::unlock`](super::ViewHandle::unlock). The offer has to allow an unlock, and
    /// the node has to be set up to approve one: on any other session the unlocker is never used.
    #[must_use]
    pub fn with_unlocker(mut self, unlocker: impl Unlocker) -> Self {
        self.unlocker = Some(Box::new(unlocker));
        self
    }

    /// The same, with the time limits of the session changed, for a test.
    #[cfg(test)]
    pub(crate) fn within(mut self, params: ViewParams) -> Self {
        self.params = Box::new(params);
        self
    }

    /// A viewer that takes what `takes` says, whether or not that is within the protocol, for the
    /// tests of the session.
    #[cfg(test)]
    pub(crate) fn raw(decoder: impl Decoder, takes: Takes, params: ViewParams) -> Self {
        Self {
            decoder: Box::new(decoder),
            speaker: None,
            unlocker: None,
            takes,
            params: Box::new(params),
        }
    }

    pub(super) fn accepted(&self, session: SessionId) -> Response {
        Response::Stream(StreamReply::Accepted(Accepted {
            session,
            max_width: self.takes.max_width,
            max_height: self.takes.max_height,
            max_fps: self.takes.max_fps,
            codecs: self.takes.codecs.clone(),
        }))
    }
}

impl fmt::Debug for Viewing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Viewing")
            .field("takes", &self.takes)
            .field("speaker", &self.speaker.is_some())
            .field("unlocker", &self.unlocker.is_some())
            .finish_non_exhaustive()
    }
}

/// What the application says to the offer of a window.
#[derive(Debug)]
pub enum WindowVerdict {
    /// Show the window.
    Accept(Viewing),
    /// The person does not want it. The source is told `Refused`.
    Refuse,
    /// The policy for the device that offered does not allow it. The source is told `Blocked`.
    Block,
}

/// What the application decides about the windows that other devices offer.
///
/// By the time [`offered`](Self::offered) is called, the offer is well-formed, the device is
/// paired, the kind of source is one this version knows, and the device has not sent more items
/// than it may. A device that was given a handler advertises `stream.view`.
pub trait WindowHandler: Send + Sync + 'static {
    /// Decides on an offer: the policy for the device that made it, and then the person, who sees
    /// what the window is. The person has two minutes, and the offer is refused when they have
    /// run out.
    ///
    /// The future is dropped when the source withdraws the offer, when the connection ends, when
    /// the device is unpaired and when the time is up, and the question is to be taken back from
    /// the person then.
    fn offered<'a>(
        &'a self,
        from: &'a PairedDevice,
        offer: &'a WindowOffer,
    ) -> BoxFuture<'a, WindowVerdict>;

    /// A window that was accepted is being shown. The handle controls the session, and dropping
    /// it ends the session. It is called before the source is told, so the application holds the
    /// handle by the time the first picture can arrive.
    fn started(&self, from: &PairedDevice, handle: ViewHandle);
}

// An application that keeps a handle to its handler hands the node a clone of it.
impl<T: WindowHandler + ?Sized> WindowHandler for Arc<T> {
    fn offered<'a>(
        &'a self,
        from: &'a PairedDevice,
        offer: &'a WindowOffer,
    ) -> BoxFuture<'a, WindowVerdict> {
        (**self).offered(from, offer)
    }

    fn started(&self, from: &PairedDevice, handle: ViewHandle) {
        (**self).started(from, handle);
    }
}

/// What a device needs to take part as a viewer.
pub(crate) struct Viewer {
    handler: Arc<dyn WindowHandler>,
    /// Whether the device may send one more item now. It counts the item.
    pub(super) charge: Box<dyn Fn(&DeviceId) -> bool + Send + Sync>,
    /// How long a person has to answer.
    pub(super) approval: Duration,
    /// Whether the device plays the sound of the screens it is shown, which is whether it
    /// advertises `stream.hear`.
    hears: bool,
    /// Who gives the key that an unlock is sealed to, if the device unlocks others with the face
    /// of its person, which is whether it advertises `unlock.approve`.
    pub(super) approver: Option<Arc<dyn UnlockApprover>>,
    /// The one question about a key that is asked at a time.
    pub(super) keying: KeyPlace,
}

impl Viewer {
    pub(crate) fn new(
        handler: Arc<dyn WindowHandler>,
        charge: impl Fn(&DeviceId) -> bool + Send + Sync + 'static,
        approval: Duration,
    ) -> Self {
        Self {
            handler,
            charge: Box::new(charge),
            approval,
            hears: false,
            approver: None,
            keying: KeyPlace::default(),
        }
    }

    /// Whether the device plays sound.
    pub(crate) fn hearing(mut self, hears: bool) -> Self {
        self.hears = hears;
        self
    }

    /// Has `approver` give the key that an unlock is sealed to.
    pub(crate) fn approving(mut self, approver: Arc<dyn UnlockApprover>) -> Self {
        self.approver = Some(approver);
        self
    }
}

/// The offer that waits for the person.
pub(super) struct Waiting {
    peer: DeviceId,
    session: SessionId,
    /// Cancelled when the source withdraws the offer, or the device is unpaired.
    withdrawn: CancellationToken,
}

/// The ids of the offers that came on a connection, oldest first.
#[derive(Default)]
pub(super) struct Seen {
    order: VecDeque<SessionId>,
    set: HashSet<SessionId>,
}

impl Seen {
    /// Remembers `session`. Returns false if it was remembered already.
    fn note(&mut self, session: SessionId) -> bool {
        if !self.set.insert(session) {
            return false;
        }
        self.order.push_back(session);
        if self.order.len() > SEEN_PER_CONNECTION
            && let Some(oldest) = self.order.pop_front()
        {
            self.set.remove(&oldest);
        }
        true
    }
}

/// What the viewer keeps about offers: the one that waits for the person, the ids seen on each
/// connection, and the connections on which the other device said what it does of the things a
/// viewer may use.
#[derive(Default)]
pub(super) struct Offers {
    pub(super) waiting: Option<Waiting>,
    pub(super) seen: HashMap<Link, Seen>,
    /// The connections whose device introduced itself as one that sends sound. A viewer sends a
    /// `Hear` only to such a device.
    pub(super) sounding: HashSet<Link>,
    /// The connections whose device introduced itself as one that can be unlocked, which a viewer
    /// that unlocks with a face is told of. A viewer sends the messages of an unlock only to such
    /// a device, and in a session whose offer said so.
    pub(super) unlocking: HashSet<Link>,
}

impl Offers {
    pub(super) fn note(&mut self, link: Link, session: SessionId) -> bool {
        self.seen.entry(link).or_default().note(session)
    }

    /// What `offer`, which came on the connection `link`, granted the viewer, and what the source
    /// on it said of itself. The offer says `unlock` only of a private session, and a viewer takes
    /// it only from a source that advertised `unlock.target`, as it advertised `unlock.approve`
    /// itself.
    pub(super) fn granted(&self, link: Link, offer: &Offer) -> Granted {
        Granted {
            control: offer.control,
            private: offer.private,
            sound: self.sounding.contains(&link),
            unlock: offer.unlock && offer.private && self.unlocking.contains(&link),
        }
    }

    /// Withdraws the offer that waits, if `doomed` says it is from a device that is gone.
    pub(super) fn withdraw_where(&self, doomed: impl Fn(&DeviceId) -> bool) {
        if let Some(waiting) = &self.waiting
            && doomed(&waiting.peer)
        {
            waiting.withdrawn.cancel();
        }
    }

    /// Withdraws the offer that waits if it is `session`, which `peer` made, and lets go of its
    /// place at once. The source is told `Stopped` next, and the offer that it makes after that
    /// must find the place free, which it would not if the place were let go of only when the
    /// task that asked the person had woken up and seen the withdrawal. That task takes the
    /// question back from the person, and sees to it that it does not let go of a later offer's
    /// place ([`Claim`]).
    fn withdraw(&mut self, peer: &DeviceId, session: SessionId) {
        if let Some(waiting) = self
            .waiting
            .take_if(|waiting| waiting.peer == *peer && waiting.session == session)
        {
            waiting.withdrawn.cancel();
        }
    }
}

/// The place of an offer in the table while it waits for the person. It is let go when this is
/// dropped, whatever ended the wait, including the future being dropped.
struct Claim<'a> {
    streaming: &'a Streaming,
    session: SessionId,
    withdrawn: CancellationToken,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        let mut table = lock(&self.streaming.table);
        if table
            .offers
            .waiting
            .as_ref()
            .is_some_and(|waiting| waiting.session == self.session)
        {
            table.offers.waiting = None;
        }
    }
}

impl Streaming {
    /// Whether this device shows the windows of other devices, which is whether it advertises
    /// `stream.view`.
    pub(crate) fn views(&self) -> bool {
        self.viewer.is_some()
    }

    /// Whether this device plays the sound of the screens it is shown, which is whether it
    /// advertises `stream.hear`.
    pub(crate) fn hears(&self) -> bool {
        self.viewer.as_ref().is_some_and(|viewer| viewer.hears)
    }

    /// Whether this device takes part in window streaming at all, which is whether it advertises
    /// one of its capabilities.
    pub(super) fn takes_part(&self) -> bool {
        self.viewer.is_some() || self.sharer.is_some()
    }

    /// The device at the other end of the connection `link` introduced itself with `hello`. A
    /// viewer is told by this which of the things it may say to a source the source takes, since
    /// the offer does not say.
    pub(crate) fn introduced(&self, link: Link, hello: &Hello) {
        let sounds = self.hears() && hello.supports(capability::STREAM_SOUND);
        let unlocks = self.approves_unlock() && hello.supports(capability::UNLOCK_TARGET);
        if sounds || unlocks {
            let mut table = lock(&self.table);
            if sounds {
                table.offers.sounding.insert(link);
            }
            if unlocks {
                table.offers.unlocking.insert(link);
            }
        }
    }

    /// Answers a request of section 9 that `paired` made on `connection`, which has the link
    /// `link`.
    pub(crate) async fn answer(
        self: &Arc<Self>,
        paired: &PairedDevice,
        connection: &Connection,
        link: Link,
        request: StreamRequest,
    ) -> StreamReply {
        match request {
            StreamRequest::Offer(offer) => self.answer_offer(paired, connection, link, offer).await,
            StreamRequest::Start(start) => self.answer_start(paired, link, start).await,
            StreamRequest::Stop(stop) => {
                // A device that takes no part has no session to stop, and says so as it does for
                // an offer.
                if !self.takes_part() {
                    return StreamReply::Declined(Declined {
                        session: stop.session,
                        reason: DeclineReason::Unsupported,
                    });
                }
                self.stop(paired, &stop);
                StreamReply::Stopped(Stopped {
                    session: stop.session,
                })
            }
        }
    }

    async fn answer_offer(
        self: &Arc<Self>,
        paired: &PairedDevice,
        connection: &Connection,
        link: Link,
        offer: Offer,
    ) -> StreamReply {
        let session = offer.session;
        let declined = |reason| StreamReply::Declined(Declined { session, reason });
        // Whether this device takes part at all, and whether it knows the kind of source.
        let Some(viewer) = &self.viewer else {
            return declined(DeclineReason::Unsupported);
        };
        if !matches!(offer.source, SourceKind::Window | SourceKind::Display) {
            return declined(DeclineReason::Unsupported);
        }
        let withdrawn = CancellationToken::new();
        {
            let mut table = lock(&self.table);
            // An id that was seen on this connection is refused, whatever came of the first offer
            // under it: nothing is repeated under a session id.
            if !table.offers.note(link, session) {
                return declined(DeclineReason::Invalid);
            }
            // The offer that a start of this device asked for is taken without asking a person:
            // the person asked when they chose to start. It is not charged to the device either,
            // since this device asked for it.
            let granted = table.offers.granted(link, &offer);
            if let Some(pending) = table
                .starting
                .take_if(|pending| pending.peer == paired.id && pending.session == session)
            {
                return self.take_asked(&mut table, paired, connection, pending, &offer, granted);
            }
            // Too many items from the device, another offer that waits for a person, a window
            // that is shown already, or a start that waits for its own offer.
            if !(viewer.charge)(&paired.id)
                || table.offers.waiting.is_some()
                || table.has_view()
                || table.starting.is_some()
            {
                return declined(DeclineReason::RateLimited);
            }
            table.offers.waiting = Some(Waiting {
                peer: paired.id,
                session,
                withdrawn: withdrawn.clone(),
            });
        }
        let claim = Claim {
            streaming: self,
            session,
            withdrawn,
        };
        let granted = lock(&self.table).offers.granted(link, &offer);

        let shown = WindowOffer::new(&offer);
        let verdict = tokio::select! {
            () = claim.withdrawn.cancelled() => None,
            verdict = tokio::time::timeout(
                viewer.approval,
                viewer.handler.offered(paired, &shown),
            ) => verdict.ok(),
        };
        match verdict {
            None | Some(WindowVerdict::Refuse) => declined(DeclineReason::Refused),
            Some(WindowVerdict::Block) => declined(DeclineReason::Blocked),
            Some(WindowVerdict::Accept(viewing)) => {
                let Response::Stream(reply) = viewing.accepted(session) else {
                    return declined(DeclineReason::Refused);
                };
                // A `Stop` that came together with the yes withdrew the offer, and the source was
                // told `Stopped`, so the answer to the offer is `Refused` (section 9). That is
                // looked at in the hold of the table lock that makes the session, and a `Stop`
                // takes the lock to withdraw an offer and to find a session, so one of the two
                // always sees the other.
                let started = {
                    let mut table = lock(&self.table);
                    if claim.withdrawn.is_cancelled() {
                        return declined(DeclineReason::Refused);
                    }
                    self.view_in(
                        &mut table,
                        paired,
                        connection.clone(),
                        session,
                        viewing,
                        granted,
                    )
                };
                // The window is shown now, and that keeps other offers out from here on.
                drop(claim);
                match started {
                    Ok(handle) => {
                        viewer.handler.started(paired, handle);
                        reply
                    }
                    Err(error) => {
                        tracing::warn!(%error, "a window could not be shown");
                        declined(DeclineReason::Invalid)
                    }
                }
            }
        }
    }

    /// Ends the session that `paired` stops, or withdraws its offer, or takes its start back, if
    /// it has one under that id. There is nothing to say if it has not: the answer is the same.
    ///
    /// A device that asked this one for an offer and has not had it takes the asking back this
    /// way, and a source that gives up on an offer it was asked for tells the device this way.
    fn stop(&self, paired: &PairedDevice, stop: &Stop) {
        let session = {
            let mut table = lock(&self.table);
            table.offers.withdraw(&paired.id, stop.session);
            if let Some(pending) = table
                .starting
                .take_if(|pending| pending.peer == paired.id && pending.session == stop.session)
            {
                let _ = pending.done.send(Err(WhyNot::Failed));
            }
            if let Some(sending) = &table.sending
                && sending.peer == paired.id
                && sending.asked == Some(stop.session)
            {
                sending.withdraw();
            }
            table
                .sessions
                .get(&stop.session)
                .filter(|session| session.peer() == paired.id)
                .cloned()
        };
        if let Some(session) = session {
            session.end(super::End::Peer);
        }
    }

    /// Lets go of what was kept for a connection that ended.
    pub(crate) fn connection_ended(&self, link: Link) {
        let mut table = lock(&self.table);
        table.offers.seen.remove(&link);
        table.offers.sounding.remove(&link);
        table.offers.unlocking.remove(&link);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(byte: u8) -> DeviceId {
        DeviceId::from_bytes([byte; 32])
    }

    fn session(byte: u8) -> SessionId {
        SessionId::from_bytes([byte; 16])
    }

    /// Offers with one that waits, made by device 1 under session 7, and the token that
    /// withdraws it.
    fn waiting() -> (Offers, CancellationToken) {
        let withdrawn = CancellationToken::new();
        let offers = Offers {
            waiting: Some(Waiting {
                peer: device(1),
                session: session(7),
                withdrawn: withdrawn.clone(),
            }),
            ..Offers::default()
        };
        (offers, withdrawn)
    }

    #[test]
    fn withdrawing_an_offer_takes_the_question_back_and_lets_go_of_its_place_at_once() {
        let (mut offers, withdrawn) = waiting();
        offers.withdraw(&device(1), session(7));
        assert!(withdrawn.is_cancelled());
        assert!(offers.waiting.is_none());
    }

    #[test]
    fn withdrawing_another_offer_or_the_offer_of_another_device_changes_nothing() {
        for (peer, id) in [(2, 7), (1, 8)] {
            let (mut offers, withdrawn) = waiting();
            offers.withdraw(&device(peer), session(id));
            assert!(!withdrawn.is_cancelled(), "device {peer}, session {id}");
            assert!(offers.waiting.is_some(), "device {peer}, session {id}");
        }
    }

    #[test]
    fn the_claim_of_a_withdrawn_offer_does_not_let_go_of_the_place_of_a_later_one() {
        let streaming = Streaming::new();
        let place = || lock(&streaming.table).offers.waiting.is_some();
        let claim = |id| Claim {
            streaming: &streaming,
            session: session(id),
            withdrawn: CancellationToken::new(),
        };
        let first = claim(7);
        // The first offer was withdrawn, and the place is taken by a later one.
        lock(&streaming.table).offers.waiting = Some(Waiting {
            peer: device(1),
            session: session(8),
            withdrawn: CancellationToken::new(),
        });
        drop(first);
        assert!(
            place(),
            "the claim of offer 7 let go of the place of offer 8"
        );
        drop(claim(8));
        assert!(!place(), "the claim of offer 8 kept its own place");
    }
}
