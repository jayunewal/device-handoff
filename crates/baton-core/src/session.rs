// SPDX-License-Identifier: Apache-2.0

//! The session protocol, `baton/1`: what paired devices say to each other.
//!
//! This module holds both directions. The server half answers requests from
//! paired devices; the client half keeps the connections this device opens to
//! them in a pool, so that handing over a second item does not pay for a second
//! handshake.
//!
//! Everything a peer sends is untrusted input even though the peer is paired:
//! frames are size-checked before they are read, requests have deadlines, and
//! items pass capability, size, rate and duplicate checks before the
//! application's [`HandoffHandler`] sees them.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use baton_proto::{
    ALPN_SESSION, DeviceId, ItemId, PROTOCOL_MINOR, close_code,
    device::sanitize_display_name,
    message::{
        DeclineReason, ErrorCode, ErrorReply, Handoff, HandoffReply, Hello, Item, Outcome, Request,
        Response, capability,
    },
    transfer::{Cancelled, Declined, TransferReply, TransferRequest},
    unlock::UnlockReply,
};
use iroh::{
    Endpoint, EndpointAddr, RelayUrl,
    endpoint::{Connection, RecvStream, SendStream},
    protocol::{AcceptError, ProtocolHandler},
};
use tokio::{
    sync::{Mutex as AsyncMutex, OnceCell},
    task::JoinSet,
};

use crate::{
    config::HandoffConfig,
    error::{Error, Result},
    gate::{code, remote_device},
    link::contact_hints,
    ratelimit::RateLimiter,
    shared::Shared,
    transfer::incoming::{FinishError, Incoming, Link},
    trust::{ContactHints, MAX_DEVICES, PairedDevice},
    util::{BoxFuture, blocking, contained, lock},
    wire::{ReadFailure, connection_error, read_message, write_message},
};

/// How long a handoff is remembered, so that a retry is recognised.
const RECENT_TTL: Duration = Duration::from_mins(10);

/// How many handoffs are remembered at most, across all devices.
const RECENT_CAPACITY: usize = 1024;

/// Longest error text sent to a peer or taken from one.
const MAX_ERROR_TEXT: usize = 200;

/// What a device does with the items other devices hand it.
///
/// The handler is the application's side of the protocol. By the time
/// [`handle`](Self::handle) is called, the item is well-formed, the sender is
/// paired, the kind of item is one the handler advertised, it is within the
/// size and rate limits, and it is not a repeat of one already handled.
pub trait HandoffHandler: Send + Sync + 'static {
    /// The kinds of item this handler accepts, as capability names from
    /// [`baton_proto::message::capability`]. They are advertised to every peer,
    /// and items of any other kind are declined without calling the handler.
    fn capabilities(&self) -> &'static [&'static str];

    /// Takes an item from `from`.
    ///
    /// The returned outcome is sent back. It should say [`Outcome::Accepted`] once
    /// the item is safely stored; what happens to it after that is the
    /// receiver's business and is not reported to the sender.
    fn handle<'a>(&'a self, from: &'a PairedDevice, handoff: Handoff) -> BoxFuture<'a, Outcome>;
}

/// The handler a node has until the application sets one: it accepts nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoHandoff;

impl HandoffHandler for NoHandoff {
    fn capabilities(&self) -> &'static [&'static str] {
        &[]
    }

    fn handle<'a>(&'a self, _from: &'a PairedDevice, _handoff: Handoff) -> BoxFuture<'a, Outcome> {
        Box::pin(async { Outcome::Declined(DeclineReason::Unsupported) })
    }
}

/// What this device says in a [`Hello`].
pub(crate) fn our_hello(shared: &Shared) -> Hello {
    let mut capabilities: Vec<String> = shared
        .handler
        .capabilities()
        .iter()
        .map(|capability| (*capability).to_owned())
        .collect();
    // A device that has a folder to receive into takes the requests and the lanes of section 8.
    if shared.incoming.as_ref().is_some_and(Incoming::takes_files) {
        capabilities.push(capability::TRANSFER_FILES.to_owned());
    }
    // A device that was given someone to ask about the windows that are offered shows them.
    if shared.streaming.views() {
        capabilities.push(capability::STREAM_VIEW.to_owned());
    }
    // A device that was set up to show its own windows offers them, and may take input.
    if shared.streaming.shares() {
        capabilities.push(capability::STREAM_SHARE.to_owned());
    }
    if shared.streaming.takes_control() {
        capabilities.push(capability::STREAM_CONTROL.to_owned());
    }
    // A device that keeps its screen private while another device controls it, and shows it when
    // that device asks.
    if shared.streaming.keeps_private() {
        capabilities.push(capability::STREAM_PRIVATE.to_owned());
    }
    // A device that was set up to send the sound of the screen it shares, or to play the sound of
    // one it is shown, says so, so that the other side may use the messages and the streams.
    if shared.streaming.sounds() {
        capabilities.push(capability::STREAM_SOUND.to_owned());
    }
    if shared.streaming.hears() {
        capabilities.push(capability::STREAM_HEAR.to_owned());
    }
    // A device that was given someone to get ready when a paired device asks it to share starts
    // the session, instead of waiting to be told which window to offer.
    if shared.streaming.starts() {
        capabilities.push(capability::STREAM_START.to_owned());
    }
    Hello {
        protocol_minor: PROTOCOL_MINOR,
        device: shared.info.clone(),
        capabilities,
    }
}

// ---------------------------------------------------------------------------
// Receiving
// ---------------------------------------------------------------------------

type Key = (DeviceId, ItemId);

struct Entry {
    seq: u64,
    at: Instant,
    cell: Arc<OnceCell<Outcome>>,
}

/// The handoffs handled lately, so that a retry gets the original answer instead
/// of being handled again.
#[derive(Default)]
struct Recent {
    next_seq: u64,
    entries: HashMap<Key, Entry>,
    /// Insertion order. An element is stale if the entry for its key has a different `seq`.
    order: VecDeque<(Key, u64)>,
}

impl Recent {
    fn find(&self, key: &Key, now: Instant) -> Option<Arc<OnceCell<Outcome>>> {
        let entry = self.entries.get(key)?;
        (now.saturating_duration_since(entry.at) < RECENT_TTL).then(|| Arc::clone(&entry.cell))
    }

    fn add(&mut self, key: Key, now: Instant) -> Arc<OnceCell<Outcome>> {
        self.purge(now);
        while self.entries.len() >= RECENT_CAPACITY {
            let Some((old, seq)) = self.order.pop_front() else {
                break;
            };
            if self.entries.get(&old).is_some_and(|entry| entry.seq == seq) {
                self.entries.remove(&old);
            }
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        let cell = Arc::new(OnceCell::new());
        self.entries.insert(
            key,
            Entry {
                seq,
                at: now,
                cell: Arc::clone(&cell),
            },
        );
        self.order.push_back((key, seq));
        cell
    }

    /// Drops what has expired or been replaced, oldest first.
    fn purge(&mut self, now: Instant) {
        while let Some((key, seq)) = self.order.front() {
            match self.entries.get(key) {
                Some(entry) if entry.seq == *seq => {
                    if now.saturating_duration_since(entry.at) < RECENT_TTL {
                        break;
                    }
                    self.entries.remove(key);
                }
                // Replaced by a newer entry for the same key, or already evicted.
                _ => {}
            }
            self.order.pop_front();
        }
    }
}

/// Checks items that arrive from paired devices before the application sees them.
pub(crate) struct Intake {
    max_clipboard_bytes: usize,
    limiter: Arc<Mutex<RateLimiter<DeviceId>>>,
    recent: Mutex<Recent>,
}

impl Intake {
    pub(crate) fn new(config: &HandoffConfig) -> Self {
        Self {
            max_clipboard_bytes: config.max_clipboard_bytes,
            limiter: Arc::new(Mutex::new(RateLimiter::new(
                config.max_items_per_minute,
                Duration::from_mins(1),
                MAX_DEVICES,
            ))),
            recent: Mutex::new(Recent::default()),
        }
    }

    /// Forgets what is known about a device's recent behaviour, once it is unpaired.
    pub(crate) fn forget(&self, peer: &DeviceId) {
        lock(&self.limiter).forget(peer);
    }

    /// What charges a transfer to the device that offers it, as one item against the same limit as
    /// the items that are handed over (section 8, Offer). It says whether the device may send one
    /// more.
    pub(crate) fn item_charge(&self) -> impl Fn(&DeviceId) -> bool + Send + Sync + 'static {
        let limiter = Arc::clone(&self.limiter);
        move |peer| lock(&limiter).allow(peer, Instant::now())
    }

    pub(crate) async fn handle(
        &self,
        handler: &dyn HandoffHandler,
        from: &PairedDevice,
        handoff: Handoff,
    ) -> HandoffReply {
        let id = handoff.id;
        let outcome = self.decide(handler, from, handoff, Instant::now()).await;
        HandoffReply { id, outcome }
    }

    async fn decide(
        &self,
        handler: &dyn HandoffHandler,
        from: &PairedDevice,
        handoff: Handoff,
        now: Instant,
    ) -> Outcome {
        if !handler
            .capabilities()
            .contains(&handoff.item.required_capability())
        {
            return Outcome::Declined(DeclineReason::Unsupported);
        }
        if let Item::ClipboardText(clip) = &handoff.item
            && clip.text.len() > self.max_clipboard_bytes
        {
            return Outcome::Declined(DeclineReason::TooLarge);
        }

        let key = (from.id, handoff.id);
        let cell = {
            let mut recent = lock(&self.recent);
            if let Some(cell) = recent.find(&key, now) {
                // A repeat of an item already seen. If it has been handled, say
                // what happened then; if the first copy is still being handled,
                // wait for it below. Repeats are not charged to the rate limit:
                // a retry is the sender doing what it should.
                if let Some(outcome) = cell.get() {
                    return *outcome;
                }
                cell
            } else {
                if !lock(&self.limiter).allow(&from.id, now) {
                    return Outcome::Declined(DeclineReason::RateLimited);
                }
                recent.add(key, now)
            }
        };
        // Only one caller runs the handler for an item; the others get its answer.
        *cell.get_or_init(|| handler.handle(from, handoff)).await
    }
}

/// Serves `baton/1` connections.
#[derive(Debug, Clone)]
pub(crate) struct SessionServer {
    pub(crate) shared: Arc<Shared>,
}

impl ProtocolHandler for SessionServer {
    async fn accept(&self, connection: Connection) -> std::result::Result<(), AcceptError> {
        // The gate has checked that the device is paired. It may open unidirectional streams too.
        let link = Link::mint();
        self.shared.uni.spawn(connection.clone(), link);
        contained(
            "session connection",
            serve_connection(Arc::clone(&self.shared), connection, link),
        )
        .await;
        Ok(())
    }
}

async fn serve_connection(shared: Arc<Shared>, connection: Connection, link: Link) {
    let peer = remote_device(&connection);
    let mut streams = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            () = shared.stop.cancelled() => {
                connection.close(code(close_code::INTERNAL), b"shutting down");
                break;
            }
            accepted = connection.accept_bi() => {
                // An error means the connection ended: closed by the peer, by us
                // because the device was unpaired, or by a timeout.
                let Ok((send, recv)) = accepted else { break };
                streams.spawn(serve_stream(
                    Arc::clone(&shared),
                    connection.clone(),
                    link,
                    peer,
                    send,
                    recv,
                ));
            }
            Some(finished) = streams.join_next(), if !streams.is_empty() => {
                if let Err(e) = finished
                    && e.is_panic()
                {
                    tracing::error!(peer = %peer.short(), "a request handler panicked");
                }
            }
        }
    }
    // Requests still in flight are abandoned along with the connection.
    shared.streaming.connection_ended(link);
}

async fn serve_stream(
    shared: Arc<Shared>,
    connection: Connection,
    link: Link,
    peer: DeviceId,
    mut send: SendStream,
    mut recv: RecvStream,
) {
    let request = match read_message::<Request>(&mut recv, shared.tuning.request_deadline).await {
        Ok(request) => request,
        Err(ReadFailure::BadPayload(error)) => {
            // Well framed but not understood. Saying so lets the sender tell a
            // bug or a version mismatch from a network failure.
            let reply = Response::Error(ErrorReply {
                code: ErrorCode::Malformed,
                message: brief(&error.to_string()),
            });
            respond(&mut send, &reply).await;
            return;
        }
        Err(ReadFailure::BadHeader(_)) => {
            abandon(&mut send, &mut recv, close_code::PROTOCOL_VIOLATION);
            return;
        }
        Err(ReadFailure::TimedOut) => {
            abandon(&mut send, &mut recv, close_code::TIMEOUT);
            return;
        }
        Err(ReadFailure::Reset(_) | ReadFailure::Transport(_)) => return,
    };

    // The list of paired devices may have changed since the connection was accepted.
    let Some(paired) = shared.trust.get(&peer) else {
        connection.close(code(close_code::NOT_PAIRED), b"not paired");
        return;
    };

    let response = match request {
        Request::Hello(hello) => {
            shared.streaming.introduced(link, &hello);
            learn_from_hello(&shared, peer, &hello).await;
            note_contact(&shared, peer, contact_hints(&connection)).await;
            Response::Hello(our_hello(&shared))
        }
        Request::Ping(ping) => Response::Pong(ping),
        Request::Handoff(handoff) => Response::Handoff(
            shared
                .intake
                .handle(shared.handler.as_ref(), &paired, handoff)
                .await,
        ),
        Request::Transfer(request) => {
            match serve_transfer(shared.incoming.as_ref(), &paired, link, request).await {
                Answered::Reply(reply) => Response::Transfer(reply),
                Answered::NotFound => Response::Error(ErrorReply {
                    code: ErrorCode::NotFound,
                    message: "no such transfer".to_owned(),
                }),
                Answered::TimedOut => {
                    // The sender asks again, and gets the outcome when it is there.
                    abandon(&mut send, &mut recv, close_code::TIMEOUT);
                    return;
                }
            }
        }
        // A device that does not show windows declines every request about one as unsupported
        // (section 9).
        Request::Stream(request) => Response::Stream(
            shared
                .streaming
                .answer(&paired, &connection, link, request)
                .await,
        ),
        // A device that keeps no password for unlocking declines every request about one as
        // unsupported (section 9). None does yet.
        Request::Unlock(_) => Response::Unlock(UnlockReply::Declined(DeclineReason::Unsupported)),
    };
    respond(&mut send, &response).await;
}

/// How a request about a transfer is answered.
enum Answered {
    Reply(TransferReply),
    /// The transfer is not known (section 8, Finish).
    NotFound,
    /// The files did not settle in time, and the request ends without an answer.
    TimedOut,
}

/// Answers a request about a transfer that `paired` made on the connection `link`.
///
/// A device that takes no part in transfers declines every request about one as unsupported
/// (section 8). That is a device that was given nothing to ask, and one that has no folder to
/// receive into at the moment. A transfer that such a device took while it had a folder is still
/// finished and cancelled as any other.
async fn serve_transfer(
    incoming: Option<&Incoming>,
    paired: &PairedDevice,
    link: Link,
    request: TransferRequest,
) -> Answered {
    let id = request.id();
    let unsupported = || {
        Answered::Reply(TransferReply::Declined(Declined {
            id,
            reason: DeclineReason::Unsupported,
        }))
    };
    let Some(incoming) = incoming else {
        return unsupported();
    };
    match request {
        TransferRequest::Offer(offer) => Answered::Reply(incoming.offer(paired, link, offer).await),
        TransferRequest::Finish(finish) => match incoming.finish(&paired.id, link, &finish).await {
            Ok(done) => Answered::Reply(TransferReply::Done(done)),
            Err(FinishError::NotFound) if !incoming.takes_files() => unsupported(),
            Err(FinishError::NotFound) => Answered::NotFound,
            Err(FinishError::TimedOut) => Answered::TimedOut,
        },
        TransferRequest::Cancel(cancel) => {
            let known = incoming.cancel(&paired.id, cancel.id).await;
            if known || incoming.takes_files() {
                Answered::Reply(TransferReply::Cancelled(Cancelled { id }))
            } else {
                unsupported()
            }
        }
    }
}

async fn respond(send: &mut SendStream, response: &Response) {
    if write_message(send, response).await.is_ok() {
        // The peer may already be gone, which does not matter here.
        let _ = send.finish();
    }
}

fn abandon(send: &mut SendStream, recv: &mut RecvStream, error: u32) {
    let _ = recv.stop(code(error));
    let _ = send.reset(code(error));
}

/// Shortens text for the other side, never cutting a character in half.
fn brief(text: &str) -> String {
    if text.len() <= MAX_ERROR_TEXT {
        return text.to_owned();
    }
    let mut end = MAX_ERROR_TEXT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// A paired device may rename itself at any time. The trust store remembers what it now
/// calls itself and flags the change for the person; the name it is shown under stays
/// the one they gave it.
async fn learn_from_hello(shared: &Shared, peer: DeviceId, hello: &Hello) {
    let trust = shared.trust.clone();
    let now = shared.clock.now_unix();
    let info = hello.device.clone();
    let saved = blocking(move || trust.observe_info(&peer, &info, now)).await;
    if let Ok(Err(e)) | Err(e) = saved {
        tracing::warn!(error = %e, "could not save what a device says about itself");
    }
}

/// Remembers that `peer` was just reached, and where.
pub(crate) async fn note_contact(shared: &Shared, peer: DeviceId, hints: ContactHints) {
    let trust = shared.trust.clone();
    let now = shared.clock.now_unix();
    let saved = blocking(move || trust.record_contact(&peer, now, &hints)).await;
    if let Ok(Err(e)) | Err(e) = saved {
        tracing::warn!(error = %e, "could not save where a device was reached");
    }
}

// ---------------------------------------------------------------------------
// Sending
// ---------------------------------------------------------------------------

/// A connection to a paired device that has been introduced and is ready for requests.
#[derive(Clone)]
pub(crate) struct Session {
    pub(crate) connection: Connection,
    /// What the peer said about itself when the connection started.
    pub(crate) peer: Arc<Hello>,
    lease: Arc<Lease>,
}

impl Session {
    /// Keeps the connection open for as long as the returned [`Hold`] lives.
    ///
    /// A connection in the pool is closed when no request has been made on it for a while, and
    /// requests are all that the pool sees. What goes on beyond them, the lanes of a transfer or
    /// the frames of a window that is shown, makes no request, so whoever runs it holds the
    /// connection. When the last hold ends, the connection counts as used at that moment.
    pub(crate) fn hold(&self) -> Hold {
        self.lease.hold()
    }

    /// How many holds are alive.
    #[cfg(test)]
    pub(crate) fn holds(&self) -> usize {
        lock(&self.lease.0).holds
    }
}

/// What besides requests is using a pooled connection.
#[derive(Debug)]
struct Lease(Mutex<LeaseState>);

#[derive(Debug)]
struct LeaseState {
    /// How many [`Hold`]s are alive.
    holds: usize,
    /// When the last hold was let go, or when the connection was made.
    released: Instant,
}

impl Lease {
    fn new() -> Self {
        Self(Mutex::new(LeaseState {
            holds: 0,
            released: Instant::now(),
        }))
    }

    fn hold(self: &Arc<Self>) -> Hold {
        lock(&self.0).holds += 1;
        Hold(Arc::clone(self))
    }

    /// Whether the connection has been unused for `idle`: no hold is alive, and the last one
    /// was let go that long ago (or never was).
    fn unused_for(&self, idle: Duration, now: Instant) -> bool {
        let state = lock(&self.0);
        state.holds == 0 && now.saturating_duration_since(state.released) >= idle
    }
}

/// Keeps a pooled connection open while it lives. See [`Session::hold`].
#[derive(Debug)]
#[must_use = "the connection is kept open only while the hold lives"]
pub(crate) struct Hold(Arc<Lease>);

impl Drop for Hold {
    fn drop(&mut self) {
        let mut state = lock(&(self.0).0);
        state.holds = state.holds.saturating_sub(1);
        if state.holds == 0 {
            state.released = Instant::now();
        }
    }
}

struct Pooled {
    session: Session,
    used: Instant,
}

/// The connections this device has open to paired devices, one per device.
#[derive(Default)]
pub(crate) struct Pool {
    slots: Mutex<HashMap<DeviceId, Arc<AsyncMutex<Option<Pooled>>>>>,
}

impl Pool {
    /// A usable session with `peer`: the pooled one if it is still open, else a new one.
    /// The flag says whether the session was reused rather than just made.
    ///
    /// Concurrent callers for the same device share one connection attempt.
    pub(crate) async fn session(
        &self,
        endpoint: &Endpoint,
        shared: &Shared,
        peer: &PairedDevice,
    ) -> Result<(Session, bool)> {
        let slot = Arc::clone(lock(&self.slots).entry(peer.id).or_default());
        let mut pooled = slot.lock().await;
        if let Some(existing) = pooled.as_mut()
            && existing.session.connection.close_reason().is_none()
        {
            existing.used = Instant::now();
            return Ok((existing.session.clone(), true));
        }
        let session = connect(endpoint, shared, peer).await?;
        *pooled = Some(Pooled {
            session: session.clone(),
            used: Instant::now(),
        });
        Ok((session, false))
    }

    /// Forgets the pooled session with `peer`, for example after it was unpaired.
    pub(crate) fn forget(&self, peer: &DeviceId) {
        lock(&self.slots).remove(peer);
    }

    /// Closes sessions that have been unused for `idle`, and forgets empty slots.
    pub(crate) fn reap(&self, idle: Duration) {
        let now = Instant::now();
        lock(&self.slots).retain(|_, slot| {
            // A slot that is busy right now is in use.
            let Ok(mut pooled) = slot.try_lock() else {
                return true;
            };
            if let Some(existing) = pooled.as_ref()
                && (existing.session.connection.close_reason().is_some()
                    || (now.saturating_duration_since(existing.used) >= idle
                        && existing.session.lease.unused_for(idle, now)))
                && let Some(done) = pooled.take()
            {
                done.session
                    .connection
                    .close(code(close_code::NORMAL), b"idle");
            }
            pooled.is_some()
        });
    }
}

/// Where to dial `peer`: its key, plus whatever was remembered about how to reach it.
fn dial_address(peer: &PairedDevice, use_relay: bool) -> Result<EndpointAddr> {
    let key = iroh::PublicKey::from_bytes(peer.id.as_bytes()).map_err(|e| Error::protocol(&e))?;
    let mut address = EndpointAddr::new(key);
    if use_relay
        && let Some(relay) = peer
            .relay
            .as_deref()
            .and_then(|relay| relay.parse::<RelayUrl>().ok())
    {
        address = address.with_relay_url(relay);
    }
    for direct in &peer.addrs {
        address = address.with_ip_addr(*direct);
    }
    Ok(address)
}

/// Opens a session connection to `peer` and introduces this device.
pub(crate) async fn connect(
    endpoint: &Endpoint,
    shared: &Shared,
    peer: &PairedDevice,
) -> Result<Session> {
    let address = dial_address(peer, shared.relay_enabled)?;
    let connection = tokio::time::timeout(
        shared.tuning.connect_timeout,
        endpoint.connect(address, ALPN_SESSION),
    )
    .await
    .map_err(|_| Error::Timeout("connecting to the other device"))?
    .map_err(|e| Error::network("connecting to the other device", &e))?;

    // Dialing by key means the handshake has proved who answered; the check is cheap
    // and the rest of the protocol relies on it.
    if remote_device(&connection) != peer.id {
        connection.close(code(close_code::PROTOCOL_VIOLATION), b"wrong device");
        return Err(Error::protocol("a different device answered"));
    }

    let hello = Request::Hello(our_hello(shared));
    let reply = match call(&connection, &hello, shared.tuning.reply_deadline).await {
        Ok(Response::Hello(reply)) => reply,
        Ok(_) => {
            connection.close(code(close_code::PROTOCOL_VIOLATION), b"unexpected reply");
            return Err(Error::protocol("the answer to hello was not a hello"));
        }
        Err(e) => {
            connection.close(code(close_code::NORMAL), b"hello failed");
            return Err(e);
        }
    };
    note_contact(shared, peer.id, contact_hints(&connection)).await;
    // The device may open unidirectional streams on a connection that this device made too.
    shared.uni.spawn(connection.clone(), Link::mint());
    Ok(Session {
        connection,
        peer: Arc::new(reply),
        lease: Arc::new(Lease::new()),
    })
}

/// Sends one request on a fresh stream and reads its one response.
pub(crate) async fn call(
    connection: &Connection,
    request: &Request,
    deadline: Duration,
) -> Result<Response> {
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|e| connection_error("opening a stream", &e))?;
    write_message(&mut send, request).await?;
    send.finish().map_err(|e| Error::network("sending", &e))?;
    match read_message::<Response>(&mut recv, deadline).await? {
        Response::Error(error) => Err(Error::Protocol(format!(
            "the other device could not process the request ({:?}): {}",
            error.code,
            sanitize_display_name(&error.message)
        ))),
        other => Ok(other),
    }
}

/// Whether trying again on a new connection could help.
pub(crate) fn worth_retrying(error: &Error) -> bool {
    match error {
        Error::Network { .. } | Error::Timeout(_) => true,
        // A deliberate refusal will not change; anything else was a connection that went away.
        Error::Closed { code, .. } => !matches!(
            u32::try_from(*code),
            Ok(close_code::NOT_PAIRED | close_code::PROTOCOL_VIOLATION)
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use baton_proto::{
        Platform,
        message::{ClipboardText, Link, capability},
    };
    use tokio::sync::Notify;

    use super::*;
    use crate::testing::TestNode;

    fn device(byte: u8) -> PairedDevice {
        PairedDevice {
            id: DeviceId::from_bytes([byte; 32]),
            name: format!("device {byte}"),
            local_name: None,
            name_changed: false,
            platform: Platform::Linux,
            added_at: 0,
            provenance: crate::Provenance::Paired,
            last_seen_at: None,
            relay: None,
            addrs: Vec::new(),
            policy: crate::DevicePolicy::default(),
        }
    }

    fn link(id: u8) -> Handoff {
        Handoff {
            id: ItemId::from_bytes([id; 16]),
            item: Item::Link(Link {
                url: "https://example.com".into(),
                title: None,
            }),
        }
    }

    fn clipboard(id: u8, bytes: usize) -> Handoff {
        Handoff {
            id: ItemId::from_bytes([id; 16]),
            item: Item::ClipboardText(ClipboardText {
                text: "x".repeat(bytes),
            }),
        }
    }

    /// Counts how often it is called, can be made slow, and accepts only links.
    #[derive(Default)]
    struct Recorder {
        calls: AtomicUsize,
        gate: Option<Arc<Notify>>,
    }

    impl HandoffHandler for Recorder {
        fn capabilities(&self) -> &'static [&'static str] {
            &[capability::LINK, capability::CLIPBOARD_TEXT]
        }

        fn handle<'a>(
            &'a self,
            _from: &'a PairedDevice,
            _handoff: Handoff,
        ) -> BoxFuture<'a, Outcome> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                if let Some(gate) = &self.gate {
                    gate.notified().await;
                }
                Outcome::Accepted
            })
        }
    }

    struct LinksOnly(Recorder);

    impl HandoffHandler for LinksOnly {
        fn capabilities(&self) -> &'static [&'static str] {
            &[capability::LINK]
        }

        fn handle<'a>(
            &'a self,
            from: &'a PairedDevice,
            handoff: Handoff,
        ) -> BoxFuture<'a, Outcome> {
            self.0.handle(from, handoff)
        }
    }

    fn intake(per_minute: u32, max_clipboard: usize) -> Intake {
        Intake::new(&HandoffConfig {
            max_inbox_items: 10,
            max_clipboard_bytes: max_clipboard,
            max_items_per_minute: per_minute,
        })
    }

    #[tokio::test]
    async fn accepted_items_reach_the_handler() {
        let intake = intake(30, 1024);
        let handler = Recorder::default();
        let reply = intake.handle(&handler, &device(1), link(1)).await;
        assert_eq!(reply.outcome, Outcome::Accepted);
        assert_eq!(reply.id, ItemId::from_bytes([1; 16]));
        assert_eq!(handler.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn kinds_the_handler_did_not_advertise_are_declined_unseen() {
        let intake = intake(30, 1024);
        let handler = LinksOnly(Recorder::default());
        let reply = intake.handle(&handler, &device(1), clipboard(1, 5)).await;
        assert_eq!(reply.outcome, Outcome::Declined(DeclineReason::Unsupported));
        assert_eq!(handler.0.calls.load(Ordering::SeqCst), 0);

        let nothing = NoHandoff;
        let reply = intake.handle(&nothing, &device(1), link(2)).await;
        assert_eq!(reply.outcome, Outcome::Declined(DeclineReason::Unsupported));
    }

    #[tokio::test]
    async fn oversized_clipboard_text_is_declined_unseen() {
        let intake = intake(30, 100);
        let handler = Recorder::default();
        let ok = intake.handle(&handler, &device(1), clipboard(1, 100)).await;
        assert_eq!(ok.outcome, Outcome::Accepted);
        let big = intake.handle(&handler, &device(1), clipboard(2, 101)).await;
        assert_eq!(big.outcome, Outcome::Declined(DeclineReason::TooLarge));
        assert_eq!(handler.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_sender_that_is_too_fast_is_declined_and_others_are_not() {
        let intake = intake(3, 1024);
        let handler = Recorder::default();
        for i in 0..3 {
            let reply = intake.handle(&handler, &device(1), link(i)).await;
            assert_eq!(reply.outcome, Outcome::Accepted);
        }
        let reply = intake.handle(&handler, &device(1), link(9)).await;
        assert_eq!(reply.outcome, Outcome::Declined(DeclineReason::RateLimited));
        assert_eq!(handler.calls.load(Ordering::SeqCst), 3);

        let other = intake.handle(&handler, &device(2), link(10)).await;
        assert_eq!(other.outcome, Outcome::Accepted);

        // Unpairing wipes the slate, so a device that pairs again starts fresh.
        intake.forget(&device(1).id);
        let again = intake.handle(&handler, &device(1), link(11)).await;
        assert_eq!(again.outcome, Outcome::Accepted);
    }

    #[tokio::test]
    async fn a_retried_item_gets_the_first_answer_and_is_not_handled_twice() {
        let intake = intake(30, 1024);
        let handler = Recorder::default();
        let first = intake.handle(&handler, &device(1), link(1)).await;
        let second = intake.handle(&handler, &device(1), link(1)).await;
        assert_eq!(first.outcome, Outcome::Accepted);
        assert_eq!(second.outcome, Outcome::Accepted);
        assert_eq!(handler.calls.load(Ordering::SeqCst), 1);

        // The same id from another device is a different item.
        let other = intake.handle(&handler, &device(2), link(1)).await;
        assert_eq!(other.outcome, Outcome::Accepted);
        assert_eq!(handler.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn retries_do_not_use_up_the_rate_limit() {
        let intake = intake(2, 1024);
        let handler = Recorder::default();
        for _ in 0..10 {
            let reply = intake.handle(&handler, &device(1), link(1)).await;
            assert_eq!(reply.outcome, Outcome::Accepted);
        }
        let reply = intake.handle(&handler, &device(1), link(2)).await;
        assert_eq!(reply.outcome, Outcome::Accepted);
    }

    #[tokio::test]
    async fn a_repeat_that_arrives_while_the_first_is_running_waits_for_it() {
        let gate = Arc::new(Notify::new());
        let handler = Recorder {
            calls: AtomicUsize::new(0),
            gate: Some(Arc::clone(&gate)),
        };
        let intake = intake(30, 1024);
        let sender = device(1);

        let both = async {
            tokio::join!(
                intake.handle(&handler, &sender, link(1)),
                intake.handle(&handler, &sender, link(1)),
            )
        };
        let release = async {
            // Let both requests get as far as they can, then finish the handler.
            for _ in 0..10 {
                tokio::task::yield_now().await;
            }
            gate.notify_waiters();
        };
        let ((first, second), ()) = tokio::join!(both, release);
        assert_eq!(first.outcome, Outcome::Accepted);
        assert_eq!(second.outcome, Outcome::Accepted);
        assert_eq!(handler.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_memory_of_recent_items_is_bounded() {
        let mut recent = Recent::default();
        let start = Instant::now();
        let peer = DeviceId::from_bytes([1; 32]);
        for i in 0..(RECENT_CAPACITY * 3) {
            let mut id = [0u8; 16];
            id[..8].copy_from_slice(&(i as u64).to_be_bytes());
            recent.add((peer, ItemId::from_bytes(id)), start);
            assert!(recent.entries.len() <= RECENT_CAPACITY);
            assert!(
                recent.order.len() <= 2 * RECENT_CAPACITY,
                "{}",
                recent.order.len()
            );
        }
    }

    #[test]
    fn recent_items_expire() {
        let mut recent = Recent::default();
        let start = Instant::now();
        let key = (DeviceId::from_bytes([1; 32]), ItemId::from_bytes([2; 16]));
        recent.add(key, start);
        let almost = RECENT_TTL.saturating_sub(Duration::from_secs(1));
        assert!(recent.find(&key, start + almost).is_some());
        assert!(recent.find(&key, start + RECENT_TTL).is_none());

        // The expired entry is replaced, not duplicated, when the item comes again.
        let again = recent.add(key, start + RECENT_TTL);
        assert!(again.get().is_none());
        assert_eq!(recent.entries.len(), 1);
        assert!(recent.find(&key, start + RECENT_TTL).is_some());
    }

    #[test]
    fn a_lease_is_unused_only_after_the_last_hold_and_for_as_long_as_the_idle_time() {
        let idle = Duration::from_secs(120);
        let lease = Arc::new(Lease::new());
        let made = Instant::now();
        assert!(!lease.unused_for(idle, made + Duration::from_secs(119)));
        assert!(lease.unused_for(idle, made + idle));

        let first = lease.hold();
        let second = lease.hold();
        let much_later = made + idle * 10;
        assert!(
            !lease.unused_for(idle, much_later),
            "held, however long it is"
        );
        drop(first);
        assert!(!lease.unused_for(idle, much_later), "one hold is left");
        let before = Instant::now();
        drop(second);
        // The time is counted from when the last hold was let go.
        let released = lock(&lease.0).released;
        assert!(released >= before);
        assert!(!lease.unused_for(idle, released + Duration::from_secs(119)));
        assert!(lease.unused_for(idle, released + idle));
    }

    #[tokio::test]
    async fn a_held_connection_is_not_closed_as_idle_and_is_when_the_hold_ends() {
        let idle = Duration::from_millis(600);
        let a = TestNode::with(|builder| {
            builder.tuning(crate::Tuning {
                idle_connection: idle,
                ..crate::Tuning::default()
            })
        })
        .await;
        let b = TestNode::start().await;
        a.pair_with(&b);

        a.node.ping(&b.id).await.unwrap();
        let session = a.node.pooled_session(&b.id).await.unwrap();
        let hold = session.hold();
        tokio::time::sleep(idle * 4).await;
        assert!(
            session.connection.close_reason().is_none(),
            "closed while held"
        );
        assert_eq!(b.node.connected_peers(), vec![a.id]);

        // Letting go counts as use: the idle time starts again, and the pool closes it after that.
        drop(hold);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            session.connection.close_reason().is_none(),
            "closed at once"
        );
        tokio::time::timeout(Duration::from_secs(10), session.connection.closed())
            .await
            .expect("an unused connection was never closed");
        // And a request after that makes a new one.
        a.node.ping(&b.id).await.unwrap();
        let again = a.node.pooled_session(&b.id).await.unwrap();
        assert_ne!(again.connection.stable_id(), session.connection.stable_id());
    }

    #[tokio::test]
    async fn a_connection_that_ends_is_forgotten_whether_or_not_it_is_held() {
        let idle = Duration::from_millis(300);
        let a = TestNode::with(|builder| {
            builder.tuning(crate::Tuning {
                idle_connection: idle,
                ..crate::Tuning::default()
            })
        })
        .await;
        let b = TestNode::start().await;
        a.pair_with(&b);

        a.node.ping(&b.id).await.unwrap();
        let session = a.node.pooled_session(&b.id).await.unwrap();
        let _hold = session.hold();
        session.connection.close(code(close_code::NORMAL), b"done");
        tokio::time::sleep(idle * 3).await;
        let again = a.node.pooled_session(&b.id).await.unwrap();
        assert!(again.connection.close_reason().is_none());
        assert_ne!(again.connection.stable_id(), session.connection.stable_id());
    }

    #[test]
    fn error_text_is_cut_on_a_character_boundary() {
        let text = "é".repeat(MAX_ERROR_TEXT);
        let short = brief(&text);
        assert!(short.len() <= MAX_ERROR_TEXT);
        assert!(short.chars().all(|c| c == 'é'));
        assert_eq!(brief("short"), "short");
    }

    #[test]
    fn only_connection_trouble_is_worth_retrying() {
        assert!(worth_retrying(&Error::Timeout("x")));
        assert!(worth_retrying(&Error::network("x", "y")));
        let closed = |code| Error::Closed {
            code,
            reason: String::new(),
        };
        assert!(worth_retrying(&closed(0)));
        assert!(worth_retrying(&closed(500)));
        assert!(!worth_retrying(&closed(403)));
        assert!(!worth_retrying(&closed(400)));
        assert!(!worth_retrying(&Error::Unsupported("x".into())));
        assert!(!worth_retrying(&Error::protocol("x")));
    }
}
