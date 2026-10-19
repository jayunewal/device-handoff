// SPDX-License-Identifier: Apache-2.0

//! [`App`]: one running Baton on one device.

use std::{
    collections::HashMap,
    fmt,
    path::PathBuf,
    str::FromStr as _,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use baton_core::{
    Config, DataDir, DirLock, Identity, Inbox, InboxEntry, InboxHandler, Node, PairApprovalRequest,
    PairedDevice, TrustStore,
    error::{Error as Core, OfferEnd},
    transfer::{destination::Destination, send::Selection},
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
    Error, ErrorKind, Event, Events, FileSend, Result,
    error::unusable_folder,
    files::Files,
    model::{
        Device, Me, Network, Offer, OfferOutcome, PairingRequest, Reach, Received, Receiving,
        TransferRequest,
    },
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
    /// Switches asking the router to open a port on or off for this run, whatever the
    /// settings file says. `None` leaves the settings file in charge.
    ///
    /// The ask starts with a multicast message too, so an iPhone or iPad app passes
    /// `Some(false)` here as well.
    pub port_mapping: Option<bool>,
    /// The folder that files sent by other devices are put in. It has to be there already.
    ///
    /// Without one this device takes no files: it does not say that it can, and a device that
    /// offers files to it is told so. With one, every transfer is put to the person first (see
    /// [`Event::TransferRequested`]), and nothing is written to the folder until they say yes.
    /// What is being received is kept in a hidden folder of its own inside this one, and each
    /// file is moved into place only when all of it has arrived and been checked, so the folder
    /// never holds half a file. A file that has the name of one already there gets a number
    /// added to its name, and nothing is replaced.
    ///
    /// The app has to be allowed to write to the folder. If Baton cannot use it, the app still
    /// starts and [`App::receiving`] says why this device takes no files.
    pub receive_dir: Option<PathBuf>,
}

impl Options {
    /// Options for the folder `data_dir`, with everything else as the settings say.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            lan_discovery: None,
            port_mapping: None,
            receive_dir: None,
        }
    }

    /// The settings from the file, with the choices made for this run on top.
    fn applied_to(&self, mut config: Config) -> Config {
        if let Some(lan_discovery) = self.lan_discovery {
            config.network.lan_discovery = lan_discovery;
        }
        if let Some(port_mapping) = self.port_mapping {
            config.network.port_mapping = port_mapping;
        }
        config
    }
}

/// One running Baton on one device.
///
/// It can be cloned: every clone is the same running Baton. Dropping the last clone stops
/// it, whatever is going on, but [`stop`](Self::stop) does so politely, and is what to call
/// when the app is done.
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

/// The questions put to the person and the answers they are waiting for.
///
/// This is kept apart from [`Inner`] because the pairing connection that asked a question
/// holds on to it for as long as the question is open. If it held the whole app, an app that
/// was dropped while a question was showing would stay running, with its folder locked,
/// until the question was over.
///
/// Pairing requests and requests for files are numbered from the same count, but they wait in
/// tables of their own, so an answer to one kind cannot reach the other.
pub(crate) struct Questions {
    /// Pairing requests that are waiting for the person's answer.
    waiting: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
    /// Requests for files that are waiting for the person's answer.
    transfers: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
    next: AtomicU64,
    events: broadcast::Sender<Event>,
}

struct Inner {
    node: Arc<Node>,
    id: DeviceId,
    name: String,
    trust: TrustStore,
    inbox: Inbox,
    events: broadcast::Sender<Event>,
    /// Cancelled as soon as stopping begins, or when the last copy of the app is dropped.
    /// Everything that goes on in the background, such as an offer that is showing, has a
    /// token that is a child of this one, so that nothing outlives the app.
    stopped: CancellationToken,
    /// Set by the first [`App::stop`], which does the work; later calls wait for it.
    stopping: AtomicBool,
    /// Cancelled when stopping is over: the node is down and the folder is free.
    released: CancellationToken,
    questions: Arc<Questions>,
    /// Whether this device takes files, which was decided when it started.
    receiving: Receiving,
    /// Held while an offer is made, so that offers reach the node in the order they were
    /// asked for.
    offer_turn: tokio::sync::Mutex<()>,
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
        let config = options.applied_to(Config::load(&dir.config_file()).map_err(starting)?);
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

        let (events, _) = broadcast::channel(EVENT_BACKLOG);
        let stopped = CancellationToken::new();
        // Made before the node, because the part of it that takes files puts its questions to the
        // person through it.
        let questions = Arc::new(Questions {
            waiting: Mutex::new(HashMap::new()),
            transfers: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            events: events.clone(),
        });

        let mut builder = Node::builder(&identity, trust.clone(), config.clone())
            .handoff_handler(InboxHandler::new(inbox.clone()));
        // The folder is opened once for the run: opening it again would remove what the transfers
        // that are going on hold. A folder that cannot be used leaves the app running, and
        // `receiving` says why it takes no files.
        let receiving = match options.receive_dir.as_deref() {
            None => Receiving::Off,
            Some(folder) => match Destination::open(folder, &identity.device_id()) {
                Ok(destination) => {
                    builder =
                        builder.transfer_handler(Files::new(destination, Arc::clone(&questions)));
                    Receiving::On
                }
                Err(e) => {
                    let reason = unusable_folder(&e);
                    tracing::warn!(%reason, "no files are taken");
                    Receiving::Unusable(reason)
                }
            },
        };
        let node = builder.bind().await.map_err(starting)?;

        tracing::info!(device = %identity.device_id().short(), "Baton started");
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
                stopping: AtomicBool::new(false),
                released: CancellationToken::new(),
                questions,
                receiving,
                offer_turn: tokio::sync::Mutex::new(()),
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
    ///
    /// It can be called more than once, from anywhere: every call returns once the folder
    /// is free, and none leaves the app half stopped if its caller stops waiting.
    pub async fn stop(&self) {
        let inner = &self.inner;
        if !inner.stopping.swap(true, Ordering::AcqRel) {
            // Calls made from here on are refused at once, not when the work below gets
            // its turn.
            inner.stopped.cancel();
            // The work runs on a task of its own, so that a caller that gives up waiting
            // does not leave it undone.
            tokio::spawn(Arc::clone(inner).shut_down());
        }
        inner.released.cancelled().await;
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
        self.running()?;
        // One offer is made at a time. Making one can take a moment (the node gives the
        // relay a few seconds to answer, so that the ticket works from other networks), and
        // two calls in that moment, a double tap say, must reach the node in the order they
        // were made, or the older offer would replace the newer one.
        let _turn = self.inner.offer_turn.lock().await;
        let node = self.running()?;
        let generation = self.inner.next_generation.fetch_add(1, Ordering::Relaxed);
        let cancel = self.inner.stopped.child_token();
        // Installed before the node is asked, so that the end of the offer it replaces
        // is recognised as old news.
        let replaced = locked(&self.inner.offer).replace(Open {
            generation,
            cancel: cancel.clone(),
        });
        if let Some(old) = replaced {
            old.cancel.cancel();
        }

        let questions = Arc::clone(&self.inner.questions);
        let approver = move |request: PairApprovalRequest| Arc::clone(&questions).ask(request);
        let offer = match node.offer_pairing(approver).await {
            Ok(offer) => offer,
            Err(e) => {
                self.inner.forget_offer(generation);
                return Err(Error::pairing(&e));
            }
        };
        // The app may have been stopped while the node was making the offer. Dropping the
        // offer withdraws it, and nobody is handed a ticket that is already dead.
        self.running()?;
        let shown = Offer {
            ticket: offer.ticket_text().to_owned(),
            expires_at: offer.expires_at(),
        };
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            // What the node decided is what is reported. A withdrawal that comes just after
            // the person said yes is too late if the node had claimed the yes by then, and
            // the pairing is reported as it happened. If the withdrawal got there first,
            // the yes is lost, and the offer is reported as withdrawn.
            let outcome = outcome_of(offer.wait_or_withdraw(cancel.cancelled_owned()).await);
            if let Some(inner) = weak.upgrade() {
                inner.finish_offer(generation, outcome);
            }
        });
        Ok(shown)
    }

    /// Withdraws the offer that is showing, if there is one. [`Event::OfferEnded`] follows.
    ///
    /// Do not call this just after [`answer_pairing`](Self::answer_pairing) with a yes: the
    /// offer ends by itself then, and a withdrawal that reaches the node before the yes
    /// does, wins. [`Event::OfferEnded`] says which it was.
    pub fn cancel_offer(&self) {
        if let Some(open) = locked(&self.inner.offer).as_ref() {
            open.cancel.cancel();
        }
    }

    /// Answers a [`PairingRequest`]. Returns `false` if it is not waiting any more, because
    /// it was answered already or the offer ended.
    ///
    /// `true` means the answer was passed on, not that the devices paired: the pairing is
    /// saved a moment later, and [`Event::OfferEnded`] reports how it went.
    pub fn answer_pairing(&self, request: u64, accept: bool) -> bool {
        let waiting = locked(&self.inner.questions.waiting).remove(&request);
        waiting.is_some_and(|answer| answer.send(accept).is_ok())
    }

    /// Whether this device takes files from other devices, and if not, why. It is decided when
    /// the app starts, from [`Options::receive_dir`].
    pub fn receiving(&self) -> Receiving {
        self.inner.receiving.clone()
    }

    /// Answers a [`TransferRequest`]. Returns `false` if it is not waiting any more, because it
    /// was answered already, the other device gave up, or the person's two minutes ran out.
    ///
    /// `true` means the answer was passed on. With a yes the files start to arrive, and
    /// [`Event::TransferEnded`] reports how it went. A no, like no answer, tells the other
    /// device that the person did not take them.
    pub fn answer_transfer(&self, request: u64, accept: bool) -> bool {
        let waiting = locked(&self.inner.questions.transfers).remove(&request);
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
        let cancel = self.inner.stopped.child_token();
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

    /// Starts sending files and folders to a paired device, and returns at once with the transfer
    /// that is going. The person at the other end is asked whether to take them, and the
    /// transfer goes on in the background from there: show [`FileSend::progress`] now and then,
    /// and wait for [`FileSend::outcome`] for the end.
    ///
    /// A folder is sent with everything in it, as it is. Links inside a folder, files that
    /// cannot be read and the like are left out, and the end says how many
    /// ([`FilesSent::left_out`](crate::model::FilesSent::left_out)).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotPaired`] if `id` is not a paired device, and [`ErrorKind::BadItem`] if
    /// there is nothing to send in `paths`, or too much for one transfer. What goes wrong once
    /// the transfer has started is the [`FileSend::outcome`].
    pub async fn send_files(&self, id: &str, paths: Vec<PathBuf>) -> Result<FileSend> {
        self.running()?;
        let (peer, name) = self.paired(id)?;
        // Walking the folders reads the disk, which can take a while for a large one, so it is
        // not done on the threads that drive the network.
        let selection = tokio::task::spawn_blocking(move || Selection::walk(&paths))
            .await
            .map_err(|_| {
                Error::new(
                    ErrorKind::Other,
                    "Baton could not look at what you chose. Try again.",
                )
            })?
            .map_err(|e| Error::selection(&e))?;
        let left_out = selection.left_out().total();
        // The app may have been stopped, or the device unpaired, while the folders were walked.
        let node = self.running()?;
        let sending = node
            .send_files(&peer, selection)
            .map_err(|e| Error::reaching(&e, &name))?;
        tracing::debug!(device = %peer.short(), "files are being sent");
        Ok(FileSend::start(sending, name, left_out))
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

impl Drop for Inner {
    fn drop(&mut self) {
        // An app that is dropped without `stop` still ends what runs in the background. The
        // node cancels its own work when it is dropped, but an offer's task would otherwise
        // wait for ever for a withdrawal that nobody is left to make, and keep the stream of
        // events open with it.
        self.stopped.cancel();
    }
}

impl Inner {
    fn emit(&self, event: Event) {
        // Nobody listening is fine.
        let _ = self.events.send(event);
    }

    /// The work of [`App::stop`]: ends everything that is going on, shuts the node down and
    /// gives up the folder, in that order.
    async fn shut_down(self: Arc<Self>) {
        // Wakes the calls that wait for this, whatever becomes of the task.
        let _released = self.released.clone().drop_guard();
        if let Some(open) = locked(&self.offer).take() {
            open.cancel.cancel();
        }
        if let Some(open) = locked(&self.join).take() {
            open.cancel.cancel();
        }
        // Questions that are open become a no.
        locked(&self.questions.waiting).clear();
        locked(&self.questions.transfers).clear();
        self.node.shutdown().await;
        // Released only now that nothing is using the folder.
        locked(&self.lock).take();
        tracing::info!(device = %self.id.short(), "Baton stopped");
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

impl Questions {
    /// Puts a pairing request to the person and waits for the answer. A request that stops
    /// mattering first, because the offer ended or the other device left, drops this
    /// future, and the screen is told to take the question away.
    async fn ask(self: Arc<Self>, request: PairApprovalRequest) -> bool {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = oneshot::channel();
        locked(&self.waiting).insert(id, answer);
        let _unanswered = Unanswered {
            questions: Arc::clone(&self),
            id,
            kind: Asked::Pairing,
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

    /// Puts a request for files to the person and waits for the answer, which is no if none
    /// comes. `request` makes the request once it has its number, and the number is returned
    /// with the answer. A request that stops mattering first, because the other device gave up
    /// or the person's time ran out, drops this future, and the screen is told to take the
    /// question away.
    pub(crate) async fn ask_transfer(
        self: Arc<Self>,
        request: impl FnOnce(u64) -> TransferRequest + Send,
    ) -> (u64, bool) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = oneshot::channel();
        locked(&self.transfers).insert(id, answer);
        let _unanswered = Unanswered {
            questions: Arc::clone(&self),
            id,
            kind: Asked::Transfer,
        };
        tracing::debug!(request = id, "request for files shown");
        self.emit(Event::TransferRequested(request(id)));
        (id, answered.await.unwrap_or(false))
    }

    /// Tells the screens something. Nobody listening is fine.
    pub(crate) fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }
}

/// Which kind of question is being asked.
#[derive(Clone, Copy)]
enum Asked {
    Pairing,
    Transfer,
}

/// Tells the screen that a question is no longer waiting, if nobody answered it.
struct Unanswered {
    questions: Arc<Questions>,
    id: u64,
    kind: Asked,
}

impl Drop for Unanswered {
    fn drop(&mut self) {
        let (table, ended) = match self.kind {
            Asked::Pairing => (
                &self.questions.waiting,
                Event::PairingRequestEnded { id: self.id },
            ),
            Asked::Transfer => (
                &self.questions.transfers,
                Event::TransferRequestEnded { id: self.id },
            ),
        };
        if locked(table).remove(&self.id).is_some() {
            self.questions.emit(ended);
        }
    }
}

/// Locks `mutex`, carrying on if another thread panicked while holding it: nothing in this
/// file leaves its data half changed.
pub(crate) fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choices_for_a_run_go_on_top_of_the_settings_file() {
        let mut file = Config::default();
        file.network.lan_discovery = true;
        file.network.port_mapping = true;

        let untouched = Options::new("/data").applied_to(file.clone());
        assert!(untouched.network.lan_discovery && untouched.network.port_mapping);

        let mut phone = Options::new("/data");
        phone.lan_discovery = Some(false);
        phone.port_mapping = Some(false);
        let phone = phone.applied_to(file.clone());
        assert!(!phone.network.lan_discovery && !phone.network.port_mapping);

        // Each choice stands alone.
        let mut only_ports = Options::new("/data");
        only_ports.port_mapping = Some(false);
        let only_ports = only_ports.applied_to(file);
        assert!(only_ports.network.lan_discovery && !only_ports.network.port_mapping);
    }
}
