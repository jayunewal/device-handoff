// SPDX-License-Identifier: Apache-2.0

//! Pairing: making two devices trust each other.
//!
//! One device, the *offerer*, opens an [`offer`](crate::Node::offer_pairing) and
//! shows its ticket. The other, the *joiner*, redeems it with
//! [`join_pairing`](crate::Node::join_pairing). The exchange itself is specified in
//! `docs/protocol.md`; this module is the state around it:
//!
//! - an offer has a secret, a deadline, and a [`PairApprover`] that asks a person;
//! - only one offer is open at a time, and it is used at most once: it puts one request
//!   to the person, and whatever becomes of that request (an answer, no answer, the
//!   joiner leaving) spends it;
//! - what a caller learns about a failed attempt is the same whether the proof
//!   was wrong, the offer expired, or there was no offer at all;
//! - both devices work out the same [`ConfirmationCode`] from the session and show it,
//!   and the person approves only if the two match.

use std::{
    fmt,
    future::Future,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use baton_proto::{
    ALPN_PAIR, DeviceId, DeviceInfo, Fingerprint, close_code,
    device::sanitize_display_name,
    pair::{
        CHANNEL_BINDING_LEN, ConfirmationCode, EXPORTER_LABEL, PairProof, PairRejection,
        PairRequest, PairResponse, PairSecret,
    },
    ticket::PairTicket,
};
use iroh::{
    Endpoint, EndpointAddr, RelayUrl,
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler},
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::{
    error::{Error, OfferEnd, Result},
    gate::{code, remote_device},
    link::contact_hints,
    shared::Shared,
    slots::PairSlot,
    trust::{ContactHints, PairedDevice},
    util::{BoxFuture, blocking, contained},
    wire::{ReadFailure, read_message, write_message},
};

/// How long to wait for the other side to take our answer before closing.
const DELIVERY_GRACE: Duration = Duration::from_secs(5);

/// What the person approving a pairing request is shown.
#[derive(Debug, Clone)]
pub struct PairApprovalRequest {
    /// The device asking to pair. The transport has proven it holds this key.
    pub peer: DeviceId,
    /// What the device says about itself. The name has been cleaned for display,
    /// but it is still chosen by the other side: show the fingerprint beside it.
    pub device: DeviceInfo,
    /// The code this session produces. The joining device shows its own; the person
    /// approves only if the two are the same.
    ///
    /// A stranger who got hold of the ticket cannot make its session produce the code
    /// that the person's own device shows, so this is what separates the device in
    /// their hand from someone else's. Show it prominently and say what to compare it
    /// with, because a name and a fingerprint tell a person nothing about that.
    pub confirmation: ConfirmationCode,
}

impl PairApprovalRequest {
    /// A short string for the person to read out, beside the name.
    pub fn fingerprint(&self) -> Fingerprint {
        self.peer.fingerprint()
    }
}

/// Asks a person whether a device may pair.
///
/// There is deliberately no implementation that always says yes outside of tests:
/// a ticket is a bearer secret, and this question is what protects against one
/// that leaked. A closure `|request| async move { ... }` works as an approver.
pub trait PairApprover: Send + Sync + 'static {
    /// Resolves to `true` if the person accepts the device. If it takes longer
    /// than two minutes, the request counts as declined.
    ///
    /// The future is dropped if the question stops mattering before the person answers:
    /// the offer is withdrawn, or the device that asked disconnects. A prompt on a screen
    /// should go away when that happens.
    fn approve(&self, request: PairApprovalRequest) -> BoxFuture<'_, bool>;
}

impl<F, Fut> PairApprover for F
where
    F: Fn(PairApprovalRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = bool> + Send + 'static,
{
    fn approve(&self, request: PairApprovalRequest) -> BoxFuture<'_, bool> {
        Box::pin(self(request))
    }
}

type Done = oneshot::Sender<Result<PairedDevice>>;

struct Open {
    id: u64,
    secret: PairSecret,
    /// When the offer lapses, by the monotonic clock. The wall-clock time in the ticket
    /// is only what the joiner is told; a jump of the wall clock must neither extend
    /// the offer nor end it early.
    deadline: Duration,
    approver: Arc<dyn PairApprover>,
    /// A valid request has been put to the person. It stays set until the offer ends:
    /// one offer shows one prompt.
    busy: bool,
    done: Done,
    /// Fired when the offer ends without being claimed, to abort a question to the person.
    cancel: CancellationToken,
}

#[derive(Default)]
struct Slot {
    current: Option<Open>,
    next_id: u64,
}

/// What to do with a pairing request.
pub(crate) enum Verdict {
    /// Answer `Rejected(Invalid)`: wrong proof, expired, used, or no offer.
    Invalid,
    /// A valid request is already waiting for a person.
    Busy,
    /// Ask the person.
    Approve(Approval),
}

/// A valid request that the person must now approve.
pub(crate) struct Approval {
    pub(crate) id: u64,
    pub(crate) approver: Arc<dyn PairApprover>,
    pub(crate) cancel: CancellationToken,
    pub(crate) confirmation: ConfirmationCode,
}

/// The one open offer, if any.
#[derive(Default)]
pub(crate) struct Offers {
    slot: Mutex<Slot>,
}

impl std::fmt::Debug for Offers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Offers")
            .field("open", &self.lock().current.is_some())
            .finish()
    }
}

/// A freshly opened offer.
pub(crate) struct Started {
    pub(crate) id: u64,
    pub(crate) cancel: CancellationToken,
    pub(crate) done: oneshot::Receiver<Result<PairedDevice>>,
}

impl Offers {
    fn lock(&self) -> MutexGuard<'_, Slot> {
        // Every change leaves the slot consistent, so a poisoned lock is safe to reuse.
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Opens an offer that lapses at `deadline` on the monotonic clock, ending the
    /// previous one if there was one.
    pub(crate) fn open(
        &self,
        secret: PairSecret,
        deadline: Duration,
        approver: Arc<dyn PairApprover>,
    ) -> Started {
        let (done, receiver) = oneshot::channel();
        let cancel = CancellationToken::new();
        let mut slot = self.lock();
        let id = slot.next_id;
        slot.next_id += 1;
        let previous = slot.current.replace(Open {
            id,
            secret,
            deadline,
            approver,
            busy: false,
            done,
            cancel: cancel.clone(),
        });
        drop(slot);
        if let Some(previous) = previous {
            finish(previous, OfferEnd::Cancelled);
        }
        Started {
            id,
            cancel,
            done: receiver,
        }
    }

    /// Ends offer `id` if it is still the open one and nobody has claimed it.
    pub(crate) fn end(&self, id: u64, end: OfferEnd) -> bool {
        let mut slot = self.lock();
        if slot.current.as_ref().is_none_or(|open| open.id != id) {
            return false;
        }
        let open = slot.current.take();
        drop(slot);
        open.is_some_and(|open| {
            finish(open, end);
            true
        })
    }

    /// Ends whatever offer is open.
    pub(crate) fn end_any(&self, end: OfferEnd) {
        let open = self.lock().current.take();
        if let Some(open) = open {
            finish(open, end);
        }
    }

    /// Ends offer `id` because its time is up, unless a valid request is with the person.
    ///
    /// A request that arrived before the deadline is still answered after it: the
    /// question is not taken away from someone who is in the middle of answering it, and
    /// the approval time-out decides when they have taken too long. Returns `false` while
    /// that is so, and `true` once there is nothing left to wait for, whether the offer
    /// ended just now or had already ended.
    pub(crate) fn expire(&self, id: u64) -> bool {
        let mut slot = self.lock();
        match slot.current.as_ref() {
            Some(open) if open.id == id => {
                if open.busy {
                    return false;
                }
            }
            _ => return true,
        }
        let open = slot.current.take();
        drop(slot);
        if let Some(open) = open {
            finish(open, OfferEnd::Expired);
        }
        true
    }

    /// Decides what to do with a request from `joiner` carrying `proof`. `now` is a
    /// reading of the monotonic clock.
    pub(crate) fn check(
        &self,
        offerer: &DeviceId,
        joiner: &DeviceId,
        channel_binding: &[u8; CHANNEL_BINDING_LEN],
        proof: &PairProof,
        now: Duration,
    ) -> Verdict {
        let mut slot = self.lock();
        let Some(open) = slot.current.as_mut() else {
            return Verdict::Invalid;
        };
        if now >= open.deadline {
            // Nobody new gets in after the deadline. An offer that a request is already
            // with the person for stays until they answer (see `expire`).
            if open.busy {
                return Verdict::Invalid;
            }
            let expired = slot.current.take();
            drop(slot);
            if let Some(expired) = expired {
                finish(expired, OfferEnd::Expired);
            }
            return Verdict::Invalid;
        }
        if !proof.verify(&open.secret, offerer, joiner, channel_binding) {
            return Verdict::Invalid;
        }
        // Only someone holding the secret gets to learn that a request is pending.
        if open.busy {
            return Verdict::Busy;
        }
        open.busy = true;
        Verdict::Approve(Approval {
            id: open.id,
            approver: Arc::clone(&open.approver),
            cancel: open.cancel.clone(),
            confirmation: ConfirmationCode::compute(&open.secret, offerer, joiner, channel_binding),
        })
    }

    /// Takes the offer for good, once the request that was put to the person has been
    /// settled: answered, timed out, or abandoned by the joiner. After this it can no
    /// longer be cancelled or expire, and no other request can be put to anyone; `None`
    /// means it was withdrawn or replaced while they decided.
    pub(crate) fn claim(&self, id: u64) -> Option<Done> {
        let mut slot = self.lock();
        if slot.current.as_ref().is_none_or(|open| open.id != id) {
            return None;
        }
        slot.current.take().map(|open| open.done)
    }

    /// Spends offer `id` because the request that was put to the person will not be
    /// answered: the device that asked has gone, or the task serving it was dropped. Does
    /// nothing if the request was settled or the offer has ended in the meantime.
    pub(crate) fn abandon(&self, id: u64) {
        if let Some(done) = self.claim(id) {
            let _ = done.send(Err(Error::OfferEnded(OfferEnd::Abandoned)));
        }
    }
}

fn finish(open: Open, end: OfferEnd) {
    open.cancel.cancel();
    // The receiver may have been dropped, which is fine.
    let _ = open.done.send(Err(Error::OfferEnded(end)));
}

/// An open invitation to pair, as the person who made it sees it.
///
/// Show [`ticket_text`](Self::ticket_text) (or render it as a QR code) on this
/// device, enter it on the other, and wait. Dropping the offer withdraws it.
pub struct PairOffer {
    pub(crate) ticket: PairTicket,
    /// The text form. It holds the secret, so it is wiped when the offer is dropped.
    pub(crate) text: Zeroizing<String>,
    pub(crate) id: u64,
    pub(crate) shared: Arc<Shared>,
    pub(crate) done: oneshot::Receiver<Result<PairedDevice>>,
}

impl fmt::Debug for PairOffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Nothing here may show the ticket: it is a secret until it has been used.
        f.debug_struct("PairOffer")
            .field("id", &self.id)
            .field("expires_at", &self.ticket.expires_at)
            .finish_non_exhaustive()
    }
}

impl PairOffer {
    /// The ticket to give to the other device.
    pub const fn ticket(&self) -> &PairTicket {
        &self.ticket
    }

    /// The ticket as text, starting with `BATON1:`.
    ///
    /// It contains the secret that lets a device ask to pair, so show it only to the
    /// person and keep it out of logs.
    pub fn ticket_text(&self) -> &str {
        &self.text
    }

    /// When the offer lapses, in seconds since the Unix epoch.
    pub const fn expires_at(&self) -> u64 {
        self.ticket.expires_at
    }

    /// Withdraws the offer. A request already waiting for the person is abandoned.
    pub fn cancel(&self) {
        self.shared.offers.end(self.id, OfferEnd::Cancelled);
    }

    /// Waits until a device has been paired, or the offer ends.
    ///
    /// # Errors
    ///
    /// [`Error::OfferEnded`] if the offer expired, was cancelled or replaced, the
    /// person declined, or the device that asked disconnected before they answered;
    /// another error if the pairing could not be saved.
    pub async fn wait(mut self) -> Result<PairedDevice> {
        match (&mut self.done).await {
            Ok(result) => result,
            // The sender is dropped only when the node goes away.
            Err(_) => Err(Error::Stopped),
        }
    }
}

impl Drop for PairOffer {
    fn drop(&mut self) {
        // A ticket nobody holds a handle to should not stay redeemable. This does
        // nothing once the offer has ended or been claimed.
        self.shared.offers.end(self.id, OfferEnd::Cancelled);
    }
}

/// Serves `baton-pair/1` connections.
#[derive(Debug, Clone)]
pub(crate) struct PairServer {
    pub(crate) shared: Arc<Shared>,
}

impl ProtocolHandler for PairServer {
    async fn accept(&self, connection: Connection) -> std::result::Result<(), AcceptError> {
        let server = self.clone();
        contained("pairing connection", async move {
            server.run(connection).await;
        })
        .await;
        Ok(())
    }

    async fn shutdown(&self) {
        self.shared.offers.end_any(OfferEnd::Cancelled);
    }
}

/// How serving a pairing connection ended, which decides how it is closed.
enum Exit {
    /// An answer was sent, or there was nothing to answer.
    Done,
    /// The peer broke the protocol.
    Violation,
    /// The peer, or the person at the other end, took too long.
    TooSlow,
    /// This device is shutting down.
    Stopping,
    /// A newer connection took this one's place, because it had shown nothing.
    PushedOut,
    /// The connection failed by itself, so there is nobody to tell.
    Gone,
}

impl PairServer {
    async fn run(&self, connection: Connection) {
        // Anyone may open a pairing connection, so bound how many are in progress. When
        // they are all taken, the newcomer pushes out the oldest one that has shown
        // nothing, so idle connections cannot keep a real joiner out.
        let Some(slot) = self.shared.pair_slots.admit() else {
            connection.close(code(close_code::OVERLOADED), b"busy");
            return;
        };
        let served = tokio::time::timeout(
            self.shared.tuning.pair_connection_limit(),
            self.serve(&connection, &slot),
        );
        let exit = tokio::select! {
            biased;
            () = self.shared.stop.cancelled() => Exit::Stopping,
            () = slot.pushed_out() => Exit::PushedOut,
            result = served => result.unwrap_or(Exit::TooSlow),
        };
        let closing: Option<(u32, &[u8])> = match exit {
            Exit::Done => Some((close_code::NORMAL, b"done")),
            Exit::Violation => Some((close_code::PROTOCOL_VIOLATION, b"bad request")),
            Exit::TooSlow => Some((close_code::TIMEOUT, b"too slow")),
            Exit::Stopping => Some((close_code::INTERNAL, b"shutting down")),
            Exit::PushedOut => Some((close_code::OVERLOADED, b"busy")),
            // Nobody is left to tell.
            Exit::Gone => None,
        };
        if let Some((error_code, reason)) = closing {
            connection.close(code(error_code), reason);
        }
    }

    async fn serve(&self, connection: &Connection, slot: &PairSlot) -> Exit {
        // A joiner opens its stream as soon as the handshake is done, so a connection that
        // has not delivered a request by the deadline is not going to. The deadline covers
        // opening the stream as well as sending on it: a stream with nothing written is not
        // visible here, so the wait for one is where a peer that does nothing would sit.
        let started = tokio::time::Instant::now();
        let deadline = self.shared.tuning.request_deadline;
        let Ok(opened) = tokio::time::timeout(deadline, connection.accept_bi()).await else {
            return Exit::TooSlow;
        };
        let Ok((mut send, mut recv)) = opened else {
            return Exit::Gone;
        };
        let remaining = deadline.saturating_sub(started.elapsed());
        let response = match read_message::<PairRequest>(&mut recv, remaining).await {
            Ok(request) => match self.decide(connection, slot, request).await {
                Ok(response) => response,
                Err(exit) => return exit,
            },
            // A frame header that breaks the limits is not worth an answer.
            Err(ReadFailure::BadHeader(_)) => return Exit::Violation,
            Err(ReadFailure::TimedOut) => return Exit::TooSlow,
            Err(ReadFailure::Transport(_)) => return Exit::Gone,
            // Garbage in the payload gets the same answer as a wrong proof.
            Err(ReadFailure::BadPayload(_)) => self.reject_invalid().await,
        };
        if write_message(&mut send, &response).await.is_err() || send.finish().is_err() {
            return Exit::Gone;
        }
        // Closing the connection right away could drop the answer on the floor.
        let _ = tokio::time::timeout(DELIVERY_GRACE, send.stopped()).await;
        Exit::Done
    }

    /// Every failure that is not the person's decision looks the same from outside,
    /// including how long it takes, so a caller without the secret learns nothing.
    async fn reject_invalid(&self) -> PairResponse {
        tokio::time::sleep(self.shared.tuning.invalid_delay).await;
        PairResponse::Rejected(PairRejection::Invalid)
    }

    /// Answers a request, or reports that this connection was pushed out before it could.
    async fn decide(
        &self,
        connection: &Connection,
        slot: &PairSlot,
        request: PairRequest,
    ) -> Result<PairResponse, Exit> {
        let shared = &self.shared;
        // The joiner's id is the one the transport authenticated, never one it claims.
        let joiner = remote_device(connection);
        let mut binding = [0u8; CHANNEL_BINDING_LEN];
        if connection
            .export_keying_material(&mut binding, EXPORTER_LABEL, &[])
            .is_err()
        {
            return Ok(self.reject_invalid().await);
        }

        // Judging a request can start something that cannot be taken back (a prompt on the
        // person's screen), so this connection must not be pushed out halfway. It is
        // protected first, and open to being pushed out again if the request was no good.
        if !slot.protect() {
            return Err(Exit::PushedOut);
        }
        let verdict = shared.offers.check(
            &shared.id,
            &joiner,
            &binding,
            &request.proof,
            shared.clock.monotonic(),
        );
        Ok(match verdict {
            Verdict::Invalid => {
                slot.unprotect();
                self.reject_invalid().await
            }
            Verdict::Busy => PairResponse::Rejected(PairRejection::Busy),
            Verdict::Approve(approval) => {
                let device = DeviceInfo {
                    name: sanitize_display_name(&request.device.name),
                    ..request.device
                };
                self.ask_and_pair(connection, joiner, device, approval)
                    .await
            }
        })
    }

    async fn ask_and_pair(
        &self,
        connection: &Connection,
        joiner: DeviceId,
        device: DeviceInfo,
        approval: Approval,
    ) -> PairResponse {
        let shared = &self.shared;
        let ask = PairApprovalRequest {
            peer: joiner,
            device: device.clone(),
            confirmation: approval.confirmation,
        };
        // If this task is dropped before the request is settled (the connection ran out
        // of time, the approver panicked), nothing else would end the offer: the deadline
        // leaves an offer alone while a request is with the person. Settling a request
        // below takes the offer first, which makes this a no-op.
        let _abandon_if_dropped = AbandonOnDrop {
            offers: &shared.offers,
            id: approval.id,
        };
        let answer = tokio::select! {
            biased;
            () = approval.cancel.cancelled() => Answer::Withdrawn,
            _ = connection.closed() => Answer::JoinerLeft,
            () = tokio::time::sleep(shared.tuning.approval_timeout) => Answer::TimedOut,
            yes = approval.approver.approve(ask) => if yes { Answer::Yes } else { Answer::No },
        };

        match answer {
            // `end` already told the offerer; the joiner is told what any bad ticket gets.
            Answer::Withdrawn => PairResponse::Rejected(PairRejection::Invalid),
            // A prompt that was shown and then abandoned counts as a decline that the
            // person never got to make. Giving the offer back instead would let one
            // leaked ticket raise any number of prompts, each with a fresh code, and the
            // six digits are enough only because an offer shows one.
            Answer::JoinerLeft => {
                shared.offers.abandon(approval.id);
                // Nobody is there to read it.
                PairResponse::Rejected(PairRejection::Invalid)
            }
            Answer::TimedOut | Answer::No => {
                let end = if matches!(answer, Answer::No) {
                    OfferEnd::Declined
                } else {
                    OfferEnd::Expired
                };
                if let Some(done) = shared.offers.claim(approval.id) {
                    let _ = done.send(Err(Error::OfferEnded(end)));
                }
                PairResponse::Rejected(PairRejection::Declined)
            }
            Answer::Yes => {
                // From here the offer cannot be cancelled: the person said yes.
                let Some(done) = shared.offers.claim(approval.id) else {
                    return PairResponse::Rejected(PairRejection::Invalid);
                };
                let hints = contact_hints(connection);
                match self.store(joiner, device, hints).await {
                    Ok(paired) => {
                        let _ = done.send(Ok(paired));
                        PairResponse::Accepted {
                            device: shared.info.clone(),
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "could not save a new pairing");
                        let _ = done.send(Err(e));
                        PairResponse::Rejected(PairRejection::Other)
                    }
                }
            }
        }
    }

    /// Saves the joiner as a paired device. This happens before the answer is sent:
    /// if the answer is lost, the offerer trusts a device that does not yet trust it
    /// back, which is safe and fixed by pairing again.
    async fn store(
        &self,
        joiner: DeviceId,
        device: DeviceInfo,
        hints: ContactHints,
    ) -> Result<PairedDevice> {
        let trust = self.shared.trust.clone();
        let now = self.shared.clock.now_unix();
        blocking(move || {
            trust.pair(joiner, &device, now)?;
            // Where the joiner just came from is the best guess at how to reach it again.
            trust.record_contact(&joiner, now, &hints)?;
            trust.get(&joiner).ok_or(Error::NotPaired(joiner))
        })
        .await?
    }
}

#[derive(Clone, Copy)]
enum Answer {
    Yes,
    No,
    TimedOut,
    Withdrawn,
    JoinerLeft,
}

/// Spends the offer as abandoned when dropped, unless the request has been settled.
struct AbandonOnDrop<'a> {
    offers: &'a Offers,
    id: u64,
}

impl Drop for AbandonOnDrop<'_> {
    fn drop(&mut self) {
        self.offers.abandon(self.id);
    }
}

/// Redeems `ticket`: connects to the offerer, proves possession of the secret, and
/// on acceptance remembers the offerer as paired.
///
/// `show_code` is called once with the confirmation code of this session, before the
/// request is sent, so that the person can see it while the other device asks them.
pub(crate) async fn join(
    endpoint: &Endpoint,
    shared: &Shared,
    ticket: &PairTicket,
    show_code: impl FnOnce(ConfirmationCode),
) -> Result<PairedDevice> {
    if ticket.offerer == shared.id {
        return Err(Error::OwnTicket);
    }
    let now = shared.clock.now_unix();
    // The offerer decides whether its offer is open, and the two clocks can differ by
    // minutes, so only a ticket that ran out long ago is refused without trying.
    if ticket.is_long_expired(now) {
        return Err(Error::TicketExpired);
    }

    let offerer =
        iroh::PublicKey::from_bytes(ticket.offerer.as_bytes()).map_err(|e| Error::protocol(&e))?;
    let mut address = EndpointAddr::new(offerer);
    if let Some(relay) = ticket
        .relay
        .as_deref()
        .and_then(|relay| relay.parse::<RelayUrl>().ok())
    {
        address = address.with_relay_url(relay);
    }
    for direct in &ticket.addrs {
        address = address.with_ip_addr(*direct);
    }

    let connection = tokio::time::timeout(
        shared.tuning.connect_timeout,
        endpoint.connect(address, ALPN_PAIR),
    )
    .await
    .map_err(|_| Error::Timeout("connecting to the other device"))?
    .map_err(|e| Error::network("connecting to the other device", &e))?;

    // Dialing by key means the handshake already proved who answered. Check anyway:
    // the whole scheme rests on it.
    if remote_device(&connection) != ticket.offerer {
        return Err(Error::protocol(
            "the device that answered is not the one named in the ticket",
        ));
    }

    let result = exchange(&connection, shared, ticket, show_code).await;
    // How the connection got there is the best hint for the next one; the ticket's
    // own hints fill in whatever it does not show.
    let mut hints = contact_hints(&connection);
    connection.close(code(close_code::NORMAL), b"done");
    let device = result?;
    if hints.relay.is_none() {
        hints.relay.clone_from(&ticket.relay);
    }
    if hints.addrs.is_empty() {
        hints.addrs.clone_from(&ticket.addrs);
    }

    let trust = shared.trust.clone();
    let offerer_id = ticket.offerer;
    blocking(move || {
        trust.pair(offerer_id, &device, now)?;
        trust.record_contact(&offerer_id, now, &hints)?;
        trust.get(&offerer_id).ok_or(Error::NotPaired(offerer_id))
    })
    .await?
}

/// Shows the confirmation code, sends the request and returns who the offerer says it is.
async fn exchange(
    connection: &Connection,
    shared: &Shared,
    ticket: &PairTicket,
    show_code: impl FnOnce(ConfirmationCode),
) -> Result<DeviceInfo> {
    let mut binding = [0u8; CHANNEL_BINDING_LEN];
    connection
        .export_keying_material(&mut binding, EXPORTER_LABEL, &[])
        .map_err(|_| Error::protocol("could not tie the pairing to this connection"))?;
    // The offerer works out the same code from the same inputs; nothing about it is sent.
    show_code(ConfirmationCode::compute(
        &ticket.secret,
        &ticket.offerer,
        &shared.id,
        &binding,
    ));
    let request = PairRequest {
        device: shared.info.clone(),
        proof: PairProof::compute(&ticket.secret, &ticket.offerer, &shared.id, &binding),
    };

    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|e| crate::wire::connection_error("opening a stream", &e))?;
    write_message(&mut send, &request).await?;
    send.finish().map_err(|e| Error::network("sending", &e))?;

    match read_message::<PairResponse>(&mut recv, shared.tuning.join_reply_deadline()).await? {
        PairResponse::Accepted { device } => Ok(device),
        PairResponse::Rejected(why) => Err(Error::PairingRejected(why)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> DeviceId {
        DeviceId::from_bytes([byte; DeviceId::LEN])
    }

    fn secret(byte: u8) -> PairSecret {
        PairSecret::from_bytes([byte; PairSecret::LEN])
    }

    fn approver() -> Arc<dyn PairApprover> {
        Arc::new(|_request: PairApprovalRequest| async { true })
    }

    const BINDING: [u8; CHANNEL_BINDING_LEN] = [9; CHANNEL_BINDING_LEN];

    fn proof(secret_byte: u8, joiner: u8) -> PairProof {
        PairProof::compute(&secret(secret_byte), &id(1), &id(joiner), &BINDING)
    }

    fn secs(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    fn check(offers: &Offers, secret_byte: u8, joiner: u8, now: u64) -> Verdict {
        offers.check(
            &id(1),
            &id(joiner),
            &BINDING,
            &proof(secret_byte, joiner),
            secs(now),
        )
    }

    fn approve(verdict: Verdict) -> Approval {
        match verdict {
            Verdict::Approve(approval) => approval,
            Verdict::Invalid => panic!("expected to be asked, got Invalid"),
            Verdict::Busy => panic!("expected to be asked, got Busy"),
        }
    }

    fn ended_with(mut done: oneshot::Receiver<Result<PairedDevice>>) -> OfferEnd {
        match done.try_recv() {
            Ok(Err(Error::OfferEnded(end))) => end,
            other => panic!("expected the offer to have ended, got {other:?}"),
        }
    }

    #[test]
    fn without_an_offer_everything_is_invalid() {
        let offers = Offers::default();
        assert!(matches!(check(&offers, 7, 2, 100), Verdict::Invalid));
    }

    #[test]
    fn a_valid_proof_is_put_to_the_person() {
        let offers = Offers::default();
        let _started = offers.open(secret(7), secs(1_000), approver());
        assert!(matches!(check(&offers, 7, 2, 100), Verdict::Approve(_)));
    }

    #[test]
    fn a_wrong_proof_is_invalid_and_does_not_end_the_offer() {
        let offers = Offers::default();
        let _started = offers.open(secret(7), secs(1_000), approver());
        // Wrong secret, and a proof made for another joiner.
        assert!(matches!(check(&offers, 8, 2, 100), Verdict::Invalid));
        let stolen = proof(7, 3);
        let verdict = offers.check(&id(1), &id(2), &BINDING, &stolen, secs(100));
        assert!(matches!(verdict, Verdict::Invalid));
        // A guesser cannot cancel the offer by failing, however often it fails.
        for _ in 0..50 {
            assert!(matches!(check(&offers, 8, 2, 100), Verdict::Invalid));
        }
        assert!(matches!(check(&offers, 7, 2, 100), Verdict::Approve(_)));
    }

    #[test]
    fn a_second_valid_request_waits_its_turn_and_is_never_asked() {
        let offers = Offers::default();
        let started = offers.open(secret(7), secs(1_000), approver());
        let first = approve(check(&offers, 7, 2, 100));
        assert!(matches!(check(&offers, 7, 3, 100), Verdict::Busy));
        // A caller without the secret is not told that a request is pending.
        assert!(matches!(check(&offers, 8, 3, 100), Verdict::Invalid));

        // Once the first request is settled the offer is spent: it has shown its one
        // prompt, so the second request is not asked either.
        let done = offers.claim(first.id).expect("the offer is still open");
        assert!(matches!(check(&offers, 7, 3, 100), Verdict::Invalid));
        drop((done, started));
    }

    #[test]
    fn an_offer_expires_at_its_deadline() {
        let offers = Offers::default();
        let started = offers.open(secret(7), secs(1_000), approver());
        assert!(matches!(check(&offers, 7, 2, 1_000), Verdict::Invalid));
        assert_eq!(ended_with(started.done), OfferEnd::Expired);
        // It is gone for good, not merely refused once.
        assert!(matches!(check(&offers, 7, 2, 10), Verdict::Invalid));
    }

    #[test]
    fn a_request_just_before_the_deadline_is_still_asked() {
        let offers = Offers::default();
        let _started = offers.open(secret(7), secs(1_000), approver());
        assert!(matches!(check(&offers, 7, 2, 999), Verdict::Approve(_)));
    }

    #[test]
    fn the_task_that_ends_offers_ends_one_nobody_is_answering() {
        let offers = Offers::default();
        let started = offers.open(secret(7), secs(1_000), approver());
        assert!(offers.expire(started.id));
        assert_eq!(ended_with(started.done), OfferEnd::Expired);
        // Nothing is left to wait for, however often it asks.
        assert!(offers.expire(started.id));
    }

    #[test]
    fn a_request_that_is_with_the_person_outlives_the_deadline() {
        let offers = Offers::default();
        let started = offers.open(secret(7), secs(1_000), approver());
        let approval = approve(check(&offers, 7, 2, 999));

        // The deadline passes while the person is deciding. Neither the task that ends
        // offers nor a request that arrives late may take the question away.
        assert!(!offers.expire(started.id), "the offer must be held open");
        assert!(!approval.cancel.is_cancelled());
        assert!(matches!(check(&offers, 7, 3, 1_001), Verdict::Invalid));
        // Not `Busy` either: nothing new is let in after the deadline, and a caller
        // that has the secret learns nothing it could not have learned before.
        assert!(matches!(check(&offers, 7, 2, 1_001), Verdict::Invalid));
        assert!(!approval.cancel.is_cancelled());

        // The answer still counts, and then there is nothing left to expire.
        let done = offers.claim(approval.id).expect("the offer is still open");
        assert!(offers.expire(started.id));
        drop(done);
    }

    #[test]
    fn a_request_whose_task_is_dropped_spends_the_offer() {
        let offers = Offers::default();
        let started = offers.open(secret(7), secs(1_000), approver());
        let approval = approve(check(&offers, 7, 2, 100));
        // The connection serving the request is cut off before the person answers. The
        // deadline leaves a busy offer alone, so without this nothing would ever end it.
        drop(AbandonOnDrop {
            offers: &offers,
            id: approval.id,
        });
        assert_eq!(ended_with(started.done), OfferEnd::Abandoned);
        assert!(matches!(check(&offers, 7, 3, 100), Verdict::Invalid));
    }

    #[test]
    fn a_request_that_was_settled_is_not_abandoned_afterwards() {
        let offers = Offers::default();
        let mut started = offers.open(secret(7), secs(1_000), approver());
        let approval = approve(check(&offers, 7, 2, 100));
        let done = offers.claim(approval.id).expect("the offer is still open");
        drop(AbandonOnDrop {
            offers: &offers,
            id: approval.id,
        });
        // Nothing was sent: the answer is still the settled request's to give.
        assert!(started.done.try_recv().is_err());
        let _ = done.send(Err(Error::OfferEnded(OfferEnd::Declined)));
        assert_eq!(ended_with(started.done), OfferEnd::Declined);
    }

    #[test]
    fn withdrawing_still_drops_a_question_after_the_deadline() {
        let offers = Offers::default();
        let started = offers.open(secret(7), secs(1_000), approver());
        let approval = approve(check(&offers, 7, 2, 999));
        assert!(!offers.expire(started.id));
        // The deadline does not protect an offer from the person who made it.
        assert!(offers.end(started.id, OfferEnd::Cancelled));
        assert!(approval.cancel.is_cancelled());
        assert_eq!(ended_with(started.done), OfferEnd::Cancelled);
    }

    #[test]
    fn a_newer_offer_replaces_the_older() {
        let offers = Offers::default();
        let first = offers.open(secret(7), secs(1_000), approver());
        let _second = offers.open(secret(8), secs(1_000), approver());
        assert!(first.cancel.is_cancelled());
        assert_eq!(ended_with(first.done), OfferEnd::Cancelled);
        assert!(matches!(check(&offers, 7, 2, 100), Verdict::Invalid));
        assert!(matches!(check(&offers, 8, 2, 100), Verdict::Approve(_)));
    }

    #[test]
    fn cancelling_aborts_the_question_and_the_offer() {
        let offers = Offers::default();
        let started = offers.open(secret(7), secs(1_000), approver());
        let approval = approve(check(&offers, 7, 2, 100));
        assert!(offers.end(started.id, OfferEnd::Cancelled));
        assert!(approval.cancel.is_cancelled());
        assert!(offers.claim(approval.id).is_none());
        assert_eq!(ended_with(started.done), OfferEnd::Cancelled);
        assert!(!offers.end(started.id, OfferEnd::Cancelled), "only once");
    }

    #[test]
    fn claiming_makes_the_offer_single_use_and_uncancellable() {
        let offers = Offers::default();
        let started = offers.open(secret(7), secs(1_000), approver());
        let approval = approve(check(&offers, 7, 2, 100));

        let done = offers.claim(approval.id).expect("the offer is still open");
        assert!(!offers.end(started.id, OfferEnd::Cancelled));
        assert!(offers.claim(approval.id).is_none());
        assert!(matches!(check(&offers, 7, 3, 100), Verdict::Invalid));
        assert!(!approval.cancel.is_cancelled());
        drop(done);
    }

    #[test]
    fn an_old_offers_id_cannot_end_a_newer_one() {
        let offers = Offers::default();
        let first = offers.open(secret(7), secs(1_000), approver());
        let second = offers.open(secret(8), secs(1_000), approver());
        assert!(!offers.end(first.id, OfferEnd::Expired));
        assert!(offers.claim(first.id).is_none());
        // Nothing is left of the old offer to expire, so there is nothing to wait for.
        assert!(offers.expire(first.id));
        assert!(matches!(check(&offers, 8, 2, 100), Verdict::Approve(_)));
        assert!(offers.end(second.id, OfferEnd::Cancelled));
    }

    #[test]
    fn the_person_is_shown_the_code_the_joining_device_works_out() {
        let offers = Offers::default();
        let _started = offers.open(secret(7), secs(1_000), approver());
        let approval = approve(check(&offers, 7, 2, 100));
        // The joiner has the same secret from the ticket, the same two ids and the same
        // session, so it arrives at the same six digits without being told them.
        let theirs = ConfirmationCode::compute(&secret(7), &id(1), &id(2), &BINDING);
        assert_eq!(approval.confirmation, theirs);
    }

    #[test]
    fn a_different_session_shows_a_different_code() {
        let offers = Offers::default();
        let _started = offers.open(secret(7), secs(1_000), approver());
        let other_session = [8; CHANNEL_BINDING_LEN];
        let proof = PairProof::compute(&secret(7), &id(1), &id(2), &other_session);
        let Verdict::Approve(approval) =
            offers.check(&id(1), &id(2), &other_session, &proof, secs(1))
        else {
            panic!("expected to be asked");
        };
        let first = ConfirmationCode::compute(&secret(7), &id(1), &id(2), &BINDING);
        assert_ne!(approval.confirmation, first);
    }

    #[tokio::test]
    async fn closures_work_as_approvers() {
        let yes: Arc<dyn PairApprover> =
            Arc::new(|request: PairApprovalRequest| async move { request.device.name == "Phone" });
        let request = |name: &str| PairApprovalRequest {
            peer: id(2),
            device: DeviceInfo {
                name: name.to_owned(),
                platform: baton_proto::Platform::Linux,
                app_version: "0.1.0".to_owned(),
            },
            confirmation: ConfirmationCode::compute(&secret(7), &id(1), &id(2), &BINDING),
        };
        assert!(yes.approve(request("Phone")).await);
        assert!(!yes.approve(request("Laptop")).await);
    }
}
