// SPDX-License-Identifier: Apache-2.0

//! [`App`]: one running Baton on one device.

use std::{
    collections::HashMap,
    fmt,
    path::PathBuf,
    str::FromStr as _,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
};

use baton_core::{
    Config, DataDir, DirLock, Identity, Inbox, InboxEntry, InboxHandler, Node, PairApprovalRequest,
    PairedDevice, TrustStore,
    error::{Error as Core, OfferEnd},
    trust::TrustEvent,
};
use baton_proto::{
    DeviceId, Platform as ProtoPlatform,
    message::{ClipboardText, Item, Link, Outcome},
    ticket::{PREFIX, PairTicket},
};
use tokio::sync::{broadcast, broadcast::error::RecvError, oneshot};
use tokio_util::sync::CancellationToken;

use crate::{
    Error, ErrorKind, Event, Events, Result,
    model::{Device, Me, Network, Offer, OfferOutcome, PairingRequest, Reach, Received},
};

/// How many events a slow reader can fall behind by before it is told to read everything
/// again.
const EVENT_BACKLOG: usize = 256;

/// How to start an [`App`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// The folder that holds this device's files: its identity, the devices it is paired
    /// with, its settings and its inbox. It is created if it is not there. Use a folder
    /// that belongs to the app alone and is not copied to other devices.
    pub data_dir: PathBuf,
    /// Switches finding other devices on the local network on or off for this run,
    /// whatever the settings file says. `None` leaves the settings file in charge.
    ///
    /// iOS only lets an app send and receive the multicast messages that discovery uses if
    /// Apple has approved it for that, so an iPhone or iPad app passes `Some(false)` and
    /// finds its paired devices through the internet relay instead.
    pub lan_discovery: Option<bool>,
}

impl Options {
    /// Options for the folder `data_dir`, with everything else as the settings say.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            lan_discovery: None,
        }
    }
}

/// One running Baton on one device.
///
/// It can be cloned: every clone is the same running Baton. Dropping the last clone stops
/// it, but [`stop`](Self::stop) does so politely, and is what to call when the app is done.
#[derive(Clone)]
pub struct App {
    inner: Arc<Inner>,
}

impl fmt::Debug for App {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("App")
            .field("device", &self.inner.id.short())
            .finish_non_exhaustive()
    }
}

/// An offer that is showing, or a pairing that is being joined, and how to give it up.
struct Open {
    /// Which one this is, so that the end of an older one is not mistaken for the end of
    /// this one.
    generation: u64,
    cancel: CancellationToken,
}

struct Inner {
    node: Arc<Node>,
    id: DeviceId,
    name: String,
    trust: TrustStore,
    inbox: Inbox,
    events: broadcast::Sender<Event>,
    stopped: CancellationToken,
    /// Requests that are waiting for the person's answer.
    pending: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
    next_request: AtomicU64,
    /// The offer that is showing.
    offer: Mutex<Option<Open>>,
    /// The pairing that is being joined, while the other device has not answered.
    join: Mutex<Option<Open>>,
    /// Numbers the offers and joins.
    next_generation: AtomicU64,
    /// The folder's lock. Given up by [`App::stop`], so that a new app can start at once.
    lock: Mutex<Option<DirLock>>,
}

impl App {
    /// Opens the device's folder, starts the network and begins keeping what arrives.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::AlreadyRunning`] if another Baton has the folder, [`ErrorKind::Storage`]
    /// or [`ErrorKind::WrongMachine`] if its files cannot be used, and
    /// [`ErrorKind::Network`] if the network could not be started.
    pub async fn start(options: Options) -> Result<Self> {
        let starting = |e: Core| Error::starting(&e);
        let dir = DataDir::open(&options.data_dir).map_err(starting)?;
        let mut config = Config::load(&dir.config_file()).map_err(starting)?;
        if let Some(lan_discovery) = options.lan_discovery {
            config.network.lan_discovery = lan_discovery;
        }
        let lock = dir.lock().map_err(starting)?;
        let identity = Identity::load_or_create(&dir.identity_file()).map_err(starting)?;
        let trust = TrustStore::load(dir.trust_file()).map_err(starting)?;
        // Opened before the node starts: a node that could not keep what it is sent must not
        // start.
        let inbox =
            Inbox::load(dir.inbox_file(), config.handoff.max_inbox_items).map_err(starting)?;

        // Subscribed before the node starts, so that nothing that arrives first is missed.
        let arrivals = inbox.subscribe();
        let trust_events = trust.subscribe();

        let node = Node::builder(&identity, trust.clone(), config.clone())
            .handoff_handler(InboxHandler::new(inbox.clone()))
            .bind()
            .await
            .map_err(starting)?;

        tracing::info!(device = %identity.device_id().short(), "Baton started");
        let (events, _) = broadcast::channel(EVENT_BACKLOG);
        let stopped = CancellationToken::new();
        tokio::spawn(forward_arrivals(arrivals, events.clone(), stopped.clone()));
        tokio::spawn(forward_trust_events(
            trust_events,
            events.clone(),
            stopped.clone(),
        ));

        Ok(Self {
            inner: Arc::new(Inner {
                node: Arc::new(node),
                id: identity.device_id(),
                name: config.device_name(),
                trust,
                inbox,
                events,
                stopped,
                pending: Mutex::new(HashMap::new()),
                next_request: AtomicU64::new(1),
                offer: Mutex::new(None),
                join: Mutex::new(None),
                next_generation: AtomicU64::new(1),
                lock: Mutex::new(Some(lock)),
            }),
        })
    }

    /// Stops the network, telling the devices that are connected, and lets go of the
    /// folder so that another [`App`] can start in it. Every later call fails with
    /// [`ErrorKind::Stopped`], and [`Events`] ends.
    pub async fn stop(&self) {
        let inner = &self.inner;
        if inner.stopped.is_cancelled() {
            return;
        }
        inner.stopped.cancel();
        if let Some(open) = locked(&inner.offer).take() {
            open.cancel.cancel();
        }
        if let Some(open) = locked(&inner.join).take() {
            open.cancel.cancel();
        }
        locked(&inner.pending).clear();
        inner.node.shutdown().await;
        // Released only now that nothing is using the folder.
        locked(&inner.lock).take();
        tracing::info!(device = %inner.id.short(), "Baton stopped");
    }

    /// This device.
    pub fn me(&self) -> Me {
        Me {
            id: self.inner.id.to_string(),
            name: self.inner.name.clone(),
            platform: ProtoPlatform::current().into(),
            fingerprint: self.inner.id.fingerprint().to_string(),
        }
    }

    /// Where this device can be reached right now.
    pub fn network(&self) -> Network {
        Network::from(&self.inner.node.status())
    }

    /// Events from now on. Get this before reading [`devices`](Self::devices) or
    /// [`inbox`](Self::inbox), so that nothing that happens while reading is lost.
    pub fn events(&self) -> Events {
        Events::new(self.inner.events.subscribe(), self.inner.stopped.clone())
    }

    /// The devices paired with this one.
    pub fn devices(&self) -> Vec<Device> {
        self.inner.trust.list().iter().map(Device::from).collect()
    }

    /// Forgets a paired device. It can no longer reach this one until it is paired again.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotPaired`] if `id` is not a paired device.
    pub fn unpair(&self, id: &str) -> Result<()> {
        let (peer, _) = self.paired(id)?;
        match self.inner.trust.remove(&peer) {
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err(Error::not_paired()),
            Err(e) => Err(Error::local(&e)),
        }
    }

    /// Opens an invitation to pair and returns it. Show [`Offer::ticket`] as a QR code and as
    /// text, and wait for [`Event::PairingRequested`] and then [`Event::OfferEnded`].
    ///
    /// An offer that is already showing is withdrawn. Only one device can pair with one
    /// offer, and only after the person has said yes to it.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Stopped`] if the app has stopped.
    pub async fn offer_pairing(&self) -> Result<Offer> {
        let node = self.running()?;
        let generation = self.inner.next_generation.fetch_add(1, Ordering::Relaxed);
        let cancel = CancellationToken::new();
        // Installed before the node is asked, so that the end of the offer it replaces
        // is recognised as old news.
        let replaced = locked(&self.inner.offer).replace(Open {
            generation,
            cancel: cancel.clone(),
        });
        if let Some(old) = replaced {
            old.cancel.cancel();
        }

        let weak = Arc::downgrade(&self.inner);
        let approver = {
            let weak = weak.clone();
            move |request: PairApprovalRequest| {
                let weak = weak.clone();
                async move {
                    match weak.upgrade() {
                        Some(inner) => inner.ask(request).await,
                        None => false,
                    }
                }
            }
        };
        let offer = match node.offer_pairing(approver).await {
            Ok(offer) => offer,
            Err(e) => {
                self.inner.forget_offer(generation);
                return Err(Error::pairing(&e));
            }
        };
        let shown = Offer {
            ticket: offer.ticket_text().to_owned(),
            expires_at: offer.expires_at(),
        };
        tokio::spawn(async move {
            let outcome = tokio::select! {
                result = offer.wait() => outcome_of(result),
                // Dropping the offer withdraws it.
                () = cancel.cancelled() => OfferOutcome::Cancelled,
            };
            if let Some(inner) = weak.upgrade() {
                inner.finish_offer(generation, outcome);
            }
        });
        Ok(shown)
    }

    /// Withdraws the offer that is showing, if there is one. [`Event::OfferEnded`] follows.
    pub fn cancel_offer(&self) {
        if let Some(open) = locked(&self.inner.offer).as_ref() {
            open.cancel.cancel();
        }
    }

    /// Answers a [`PairingRequest`]. Returns `false` if it is not waiting any more, because
    /// it was answered already or the offer ended.
    pub fn answer_pairing(&self, request: u64, accept: bool) -> bool {
        let waiting = locked(&self.inner.pending).remove(&request);
        waiting.is_some_and(|answer| answer.send(accept).is_ok())
    }

    /// Pairs with the device that made `ticket`, text as [`Offer::ticket`] shows it or as
    /// a camera read it from the other device's QR code. Anything around the ticket, such
    /// as the rest of a pasted command, is ignored.
    ///
    /// `show_code` is called with the six digits (as `514 643`) once the other device has
    /// been reached. Show them: the person on the other device checks that it shows the same
    /// ones, and says yes there. This returns when it has, or when it has not.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadTicket`] if the text is not a usable ticket,
    /// [`ErrorKind::Unreachable`] if the other device cannot be reached, and
    /// [`ErrorKind::PairingFailed`] if it does not pair.
    pub async fn join_pairing(
        &self,
        ticket: &str,
        show_code: impl FnOnce(String) + Send,
    ) -> Result<Device> {
        let node = self.running()?;
        let ticket = parse_ticket(ticket)?;
        let generation = self.inner.next_generation.fetch_add(1, Ordering::Relaxed);
        let cancel = CancellationToken::new();
        let earlier = locked(&self.inner.join).replace(Open {
            generation,
            cancel: cancel.clone(),
        });
        if let Some(earlier) = earlier {
            earlier.cancel.cancel();
        }
        let paired = tokio::select! {
            result = node.join_pairing(&ticket, |code| show_code(code.to_string())) => {
                result.map_err(|e| Error::pairing(&e))
            }
            () = cancel.cancelled() => Err(Error::new(
                ErrorKind::PairingFailed,
                "Pairing was cancelled.",
            )),
        };
        {
            // Whatever is in the slot now is either this call's or a newer one's.
            let mut slot = locked(&self.inner.join);
            if slot
                .as_ref()
                .is_some_and(|open| open.generation == generation)
            {
                slot.take();
            }
        }
        paired.map(|device| Device::from(&device))
    }

    /// Gives up on a [`join_pairing`](Self::join_pairing) that is still waiting.
    pub fn cancel_join(&self) {
        if let Some(open) = locked(&self.inner.join).take() {
            open.cancel.cancel();
        }
    }

    /// Checks that a paired device answers, and how its traffic gets here.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotPaired`] or [`ErrorKind::Unreachable`].
    pub async fn ping(&self, id: &str) -> Result<Reach> {
        let node = self.running()?;
        let (peer, name) = self.paired(id)?;
        node.ping(&peer)
            .await
            .map(Reach::from)
            .map_err(|e| Error::reaching(&e, &name))
    }

    /// Sends a web link to a paired device.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::BadItem`] if `url` is not a full web address, [`ErrorKind::NotPaired`],
    /// [`ErrorKind::Unreachable`] if the device cannot be reached, and
    /// [`ErrorKind::Declined`] if it answers and does not take the link.
    pub async fn send_link(&self, id: &str, url: &str, title: Option<&str>) -> Result<()> {
        let url = url.trim();
        if url::Url::parse(url).is_err() {
            return Err(Error::new(
                ErrorKind::BadItem,
                "That is not a full web address. Start it with https://",
            ));
        }
        let title = title
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_owned);
        let item = Item::Link(Link {
            url: url.to_owned(),
            title,
        });
        self.send(id, "link", item).await
    }

    /// Sends text, such as what was on the clipboard, to a paired device.
    ///
    /// # Errors
    ///
    /// As for [`send_link`](Self::send_link), with [`ErrorKind::BadItem`] for text that is
    /// empty or longer than the settings allow.
    pub async fn send_text(&self, id: &str, text: &str) -> Result<()> {
        if text.trim().is_empty() {
            return Err(Error::new(ErrorKind::BadItem, "There is no text to send."));
        }
        let item = Item::ClipboardText(ClipboardText {
            text: text.to_owned(),
        });
        self.send(id, "text", item).await
    }

    /// What other devices have sent, newest first.
    ///
    /// This reads a file. Call it from a thread where waiting a moment is fine.
    pub fn inbox(&self) -> Result<Vec<Received>> {
        self.running()?;
        let mut entries: Vec<InboxEntry> = self.inner.inbox.list().map_err(|e| Error::local(&e))?;
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.number));
        Ok(entries.iter().map(Received::from).collect())
    }

    /// Removes one item from the inbox. Returns whether there was such an item.
    ///
    /// This writes a file. Call it from a thread where waiting a moment is fine.
    pub fn remove_received(&self, number: u64) -> Result<bool> {
        self.running()?;
        self.inner
            .inbox
            .remove(number)
            .map(|removed| removed.is_some())
            .map_err(|e| Error::local(&e))
    }

    /// Removes everything from the inbox. Returns how many items it held.
    ///
    /// This writes a file. Call it from a thread where waiting a moment is fine.
    pub fn clear_inbox(&self) -> Result<u64> {
        self.running()?;
        self.inner
            .inbox
            .clear()
            .map(|count| count as u64)
            .map_err(|e| Error::local(&e))
    }

    /// Sends `item`, which is `what` to a person, to the paired device `id`.
    async fn send(&self, id: &str, what: &str, item: Item) -> Result<()> {
        let node = self.running()?;
        let (peer, name) = self.paired(id)?;
        match node.send(&peer, item).await {
            Ok(Outcome::Accepted) => Ok(()),
            Ok(Outcome::Declined(reason)) => Err(Error::declined(&name, what, reason)),
            Err(e) => Err(Error::reaching(&e, &name)),
        }
    }

    /// The running node, or the reason there is none.
    fn running(&self) -> Result<&Arc<Node>> {
        if self.inner.stopped.is_cancelled() {
            Err(Error::stopped())
        } else {
            Ok(&self.inner.node)
        }
    }

    /// The paired device `id` stands for, and the name to call it by.
    fn paired(&self, id: &str) -> Result<(DeviceId, String)> {
        let peer = DeviceId::from_str(id.trim()).map_err(|_| Error::not_paired())?;
        let device: PairedDevice = self.inner.trust.get(&peer).ok_or_else(Error::not_paired)?;
        Ok((peer, device.display_name().to_owned()))
    }
}

impl Inner {
    fn emit(&self, event: Event) {
        // Nobody listening is fine.
        let _ = self.events.send(event);
    }

    /// Puts a pairing request to the person and waits for the answer. A request that stops
    /// mattering first, because the offer ended or the other device left, drops this
    /// future, and the screen is told to take the question away.
    async fn ask(self: Arc<Self>, request: PairApprovalRequest) -> bool {
        let id = self.next_request.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = oneshot::channel();
        locked(&self.pending).insert(id, answer);
        let _unanswered = Unanswered {
            inner: Arc::clone(&self),
            id,
        };
        tracing::debug!(request = id, device = %request.peer.short(), "pairing request shown");
        self.emit(Event::PairingRequested(PairingRequest {
            id,
            device_id: request.peer.to_string(),
            name: request.device.name.clone(),
            platform: request.device.platform.into(),
            fingerprint: request.fingerprint().to_string(),
            code: request.confirmation.to_string(),
        }));
        answered.await.unwrap_or(false)
    }

    /// Takes the offer of this generation out of the slot, if it is still there.
    fn forget_offer(&self, generation: u64) -> bool {
        let mut slot = locked(&self.offer);
        if slot
            .as_ref()
            .is_some_and(|open| open.generation == generation)
        {
            slot.take();
            true
        } else {
            false
        }
    }

    /// Reports how an offer ended, unless a newer offer has taken its place.
    fn finish_offer(&self, generation: u64, outcome: OfferOutcome) {
        if self.forget_offer(generation) {
            tracing::debug!(?outcome, "pairing offer ended");
            self.emit(Event::OfferEnded(outcome));
        }
    }
}

/// Tells the screen that a pairing request is no longer waiting, if nobody answered it.
struct Unanswered {
    inner: Arc<Inner>,
    id: u64,
}

impl Drop for Unanswered {
    fn drop(&mut self) {
        if locked(&self.inner.pending).remove(&self.id).is_some() {
            self.inner.emit(Event::PairingRequestEnded { id: self.id });
        }
    }
}

/// Locks `mutex`, carrying on if another thread panicked while holding it: nothing in this
/// file leaves its data half changed.
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Reads a ticket out of what the person pasted or a camera read.
fn parse_ticket(text: &str) -> Result<PairTicket> {
    // The text may be a whole command, `baton pair join BATON1:...`. The ticket starts where
    // its prefix does; the ticket's own parser ignores case and line breaks inside it.
    let candidate = text
        .to_ascii_uppercase()
        .find(PREFIX)
        .and_then(|start| text.get(start..))
        .unwrap_or(text);
    candidate.parse::<PairTicket>().map_err(|e| {
        Error::new(
            ErrorKind::BadTicket,
            format!("That ticket does not work: {e}."),
        )
    })
}

/// How an offer ended, from what waiting for it returned.
fn outcome_of(result: baton_core::Result<PairedDevice>) -> OfferOutcome {
    match result {
        Ok(device) => OfferOutcome::Paired(Device::from(&device)),
        Err(Core::OfferEnded(end)) => match end {
            OfferEnd::Cancelled => OfferOutcome::Cancelled,
            OfferEnd::Expired => OfferOutcome::Expired,
            OfferEnd::Declined => OfferOutcome::Declined,
            OfferEnd::Abandoned => OfferOutcome::Abandoned,
            _ => OfferOutcome::Failed(Error::pairing(&Core::OfferEnded(end)).message().to_owned()),
        },
        Err(Core::Stopped) => OfferOutcome::Cancelled,
        Err(other) => OfferOutcome::Failed(Error::pairing(&other).message().to_owned()),
    }
}

/// Turns each item that arrives into an event.
async fn forward_arrivals(
    mut arrivals: broadcast::Receiver<Arc<InboxEntry>>,
    events: broadcast::Sender<Event>,
    stopped: CancellationToken,
) {
    loop {
        let event = tokio::select! {
            () = stopped.cancelled() => return,
            arrival = arrivals.recv() => match arrival {
                Ok(entry) => Event::Received(Received::from(&*entry)),
                Err(RecvError::Lagged(_)) => Event::Resync,
                Err(RecvError::Closed) => return,
            },
        };
        let _ = events.send(event);
    }
}

/// Turns each change to the list of paired devices into an event.
async fn forward_trust_events(
    mut changes: broadcast::Receiver<TrustEvent>,
    events: broadcast::Sender<Event>,
    stopped: CancellationToken,
) {
    loop {
        tokio::select! {
            () = stopped.cancelled() => return,
            change = changes.recv() => match change {
                Ok(_) | Err(RecvError::Lagged(_)) => {
                    let _ = events.send(Event::DevicesChanged);
                }
                Err(RecvError::Closed) => return,
            },
        }
    }
}
