// SPDX-License-Identifier: Apache-2.0

//! A running Baton node: the network endpoint, pairing, and sending to paired devices.
//!
//! [`NodeBuilder::bind`] starts everything a device needs to take part: a QUIC
//! endpoint keyed by the device identity, discovery on the local network, a
//! router that serves the two protocols, and the gate that keeps unpaired
//! devices out. A [`Node`] is then used to [pair](Node::offer_pairing) with
//! other devices and to [send](Node::send) them things.
//!
//! ```no_run
//! use baton_core::{Config, DataDir, Identity, Node, TrustStore};
//!
//! # async fn run() -> baton_core::Result<()> {
//! let dir = DataDir::open("/tmp/baton-example")?;
//! let _lock = dir.lock()?;
//! let identity = Identity::load_or_create(&dir.identity_file())?;
//! let trust = TrustStore::load(dir.trust_file())?;
//! let config = Config::load(&dir.config_file())?;
//!
//! let node = Node::builder(&identity, trust, config).bind().await?;
//! let offer = node
//!     .offer_pairing(|request: baton_core::PairApprovalRequest| async move {
//!         println!(
//!             "{} ({}) wants to pair. Approve only if the other device shows {}.",
//!             request.device.name,
//!             request.fingerprint(),
//!             request.confirmation
//!         );
//!         true
//!     })
//!     .await?;
//! println!("enter this on the other device: {}", offer.ticket_text());
//! let paired = offer.wait().await?;
//! println!("paired with {}", paired.name);
//! node.shutdown().await;
//! # Ok(())
//! # }
//! ```

use std::{
    fmt,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use baton_proto::{
    ALPN_PAIR, ALPN_SESSION, DeviceId, DeviceInfo, ItemId, Platform, close_code,
    message::{Handoff, Item, Message, Outcome, Ping, Request, Response},
    pair::{ConfirmationCode, PairSecret},
    ticket::{MAX_ADDRS, MAX_RELAY_BYTES, PairTicket},
};
use iroh::{
    Endpoint, EndpointAddr, RelayMode, RelayUrl, SecretKey,
    address_lookup::{DnsAddressLookup, PkarrPublisher, PkarrResolver},
    endpoint::{Builder, PortmapperConfig, VarInt, presets},
    protocol::Router,
};
use iroh_mdns_address_lookup::MdnsAddressLookup;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

#[cfg(any(test, feature = "test-support"))]
use crate::gate::BeforeRegistering;
#[cfg(test)]
use crate::uni::{Kind, Sink};
use crate::{
    clock::{Clock, SystemClock},
    config::{self, Config, NetworkConfig},
    error::{Error, OfferEnd, Result},
    gate::{Admission, ConnectionRegistry, Gate},
    identity::Identity,
    link::{PathKind, is_dialable, is_local, selected_path},
    pairing::{self, Offers, PairApprover, PairOffer, PairServer},
    session::{HandoffHandler, Intake, NoHandoff, Pool, SessionServer, call, worth_retrying},
    shared::Shared,
    slots::{MAX_PAIRING_CONNECTIONS, PairSlots},
    transport,
    trust::{PairedDevice, TrustEvent, TrustStore},
    uni::{Sinks, Uni},
    util::random_bytes,
};

/// The name devices use to find each other on the local network.
const MDNS_SERVICE: &str = "baton";

/// How long to wait for the relay before making a ticket without it.
const RELAY_WAIT: Duration = Duration::from_secs(3);

/// Time limits. The defaults suit real networks; tests shorten them.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Tuning {
    /// How long a peer has to open a stream and send its request. A pairing connection
    /// that has sent nothing by then is closed, which is what keeps its place from being
    /// held by a peer that never intends to.
    pub request_deadline: Duration,
    /// How long to wait for the answer to a request.
    pub reply_deadline: Duration,
    /// How long to wait for a connection to a device to come up.
    pub connect_timeout: Duration,
    /// How long a person has to approve a pairing request.
    pub approval_timeout: Duration,
    /// How long a refused pairing request is held before the refusal is sent.
    pub invalid_delay: Duration,
    /// How long an unused connection to a paired device is kept open.
    pub idle_connection: Duration,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            request_deadline: Duration::from_secs(10),
            reply_deadline: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(20),
            approval_timeout: Duration::from_mins(2),
            invalid_delay: Duration::from_millis(250),
            idle_connection: Duration::from_mins(2),
        }
    }
}

impl Tuning {
    /// The longest a pairing connection may stay open: the request, the person's
    /// answer, and some slack.
    pub(crate) fn pair_connection_limit(&self) -> Duration {
        self.request_deadline + self.approval_timeout + Duration::from_secs(15)
    }

    /// How long the joining side waits for an answer: the approval plus the network.
    pub(crate) fn join_reply_deadline(&self) -> Duration {
        self.approval_timeout + Duration::from_secs(30)
    }
}

/// Sets up a [`Node`].
pub struct NodeBuilder {
    secret: SecretKey,
    trust: TrustStore,
    config: Config,
    handler: Arc<dyn HandoffHandler>,
    clock: Arc<dyn Clock>,
    tuning: Tuning,
    /// What takes the unidirectional streams that paired devices open. Nothing yet: the receiver
    /// of files and window streaming are installed here as they are added.
    sinks: Sinks,
    tweak: Option<Box<dyn FnOnce(Builder) -> Builder + Send>>,
    #[cfg(any(test, feature = "test-support"))]
    before_registering: Option<BeforeRegistering>,
}

impl fmt::Debug for NodeBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeBuilder").finish_non_exhaustive()
    }
}

impl NodeBuilder {
    /// Starts from this device's identity, its list of paired devices and its settings.
    pub fn new(identity: &Identity, trust: TrustStore, config: Config) -> Self {
        Self {
            secret: identity.secret_key().clone(),
            trust,
            config,
            handler: Arc::new(NoHandoff),
            clock: Arc::new(SystemClock),
            tuning: Tuning::default(),
            sinks: Sinks::default(),
            tweak: None,
            #[cfg(any(test, feature = "test-support"))]
            before_registering: None,
        }
    }

    /// Sets what happens to items other devices hand over. Without one, all are declined.
    #[must_use]
    pub fn handoff_handler(mut self, handler: impl HandoffHandler) -> Self {
        self.handler = Arc::new(handler);
        self
    }

    /// Replaces the clock used for ticket lifetimes, mainly so tests can move time.
    #[must_use]
    pub fn clock(mut self, clock: impl Clock) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    /// Replaces the time limits.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn tuning(mut self, tuning: Tuning) -> Self {
        self.tuning = tuning;
        self
    }

    /// Has `sink` take the unidirectional streams of `kind`, at most `limit` at once on a
    /// connection.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn take_streams(mut self, kind: Kind, limit: u32, sink: Arc<dyn Sink>) -> Self {
        self.sinks = self.sinks.install(kind, limit, sink);
        self
    }

    /// Changes the network endpoint's settings last, for example to use a test relay.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn customize_endpoint(
        mut self,
        tweak: impl FnOnce(Builder) -> Builder + Send + 'static,
    ) -> Self {
        self.tweak = Some(Box::new(tweak));
        self
    }

    /// Waits for `hook` for each session connection that has passed the first check of
    /// the list of paired devices and is about to be registered, so that a test can
    /// unpair the device at exactly that moment and let the rest of the node react to it
    /// before the connection is registered.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn before_registering<Fut>(
        mut self,
        hook: impl Fn(DeviceId) -> Fut + Send + Sync + 'static,
    ) -> Self
    where
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.before_registering = Some(Arc::new(move |peer| Box::pin(hook(peer))));
        self
    }

    /// Starts the node: binds the network endpoint and begins serving.
    ///
    /// # Errors
    ///
    /// If the settings are invalid or the endpoint cannot be bound.
    pub async fn bind(self) -> Result<Node> {
        let Self {
            secret,
            trust,
            config,
            handler,
            clock,
            tuning,
            sinks,
            tweak,
            #[cfg(any(test, feature = "test-support"))]
            before_registering,
        } = self;
        config.validate()?;

        let id = DeviceId::from_bytes(*secret.public().as_bytes());
        let relay_enabled = config.network.relay != config::RelayMode::Disabled;
        let registry = Arc::new(ConnectionRegistry::default());
        let stop = CancellationToken::new();
        let uni = Arc::new(Uni::new(sinks, &trust, &stop, tuning.request_deadline));
        let shared = Arc::new(Shared {
            id,
            info: DeviceInfo {
                name: config.device_name(),
                platform: Platform::current(),
                app_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            trust: trust.clone(),
            clock,
            tuning,
            relay_enabled,
            stop,
            registry: Arc::clone(&registry),
            offers: Offers::default(),
            pair_slots: PairSlots::new(MAX_PAIRING_CONNECTIONS),
            handler,
            intake: Intake::new(&config.handoff),
            uni: Arc::clone(&uni),
        });

        let gate = Gate::new(trust.clone(), registry, uni.quic_limit());
        #[cfg(any(test, feature = "test-support"))]
        let gate = match before_registering {
            Some(hook) => gate.before_registering(hook),
            None => gate,
        };
        let mut builder = Endpoint::builder(presets::Minimal)
            .secret_key(secret)
            .relay_mode(relay_mode(&config.network)?)
            .hooks(gate)
            .transport_config(transport::config())
            .portmapper_config(portmapper_config(&config.network));
        if config.network.dns_discovery && relay_enabled {
            builder = builder
                .address_lookup(PkarrPublisher::n0_dns())
                .address_lookup(PkarrResolver::n0_dns())
                .address_lookup(DnsAddressLookup::n0_dns());
        }
        if let Some(tweak) = tweak {
            builder = tweak(builder);
        }
        let endpoint = builder
            .bind()
            .await
            .map_err(|e| Error::network("starting the network endpoint", &e))?;
        let lan_discovery = config.network.lan_discovery && start_lan_discovery(&endpoint);

        // Subscribe before the first connection can arrive, so no unpairing is missed.
        let trust_events = trust.subscribe();
        let admission = Arc::new(Admission::new(trust.clone()));
        let router = Router::builder(endpoint.clone())
            .accept(
                ALPN_SESSION,
                SessionServer {
                    shared: Arc::clone(&shared),
                },
            )
            .accept(
                ALPN_PAIR,
                PairServer {
                    shared: Arc::clone(&shared),
                },
            )
            .incoming_filter(Arc::new(move |incoming| admission.judge(incoming)))
            .spawn();

        let pool = Arc::new(Pool::default());
        tokio::spawn(watch_trust(
            Arc::clone(&shared),
            Arc::clone(&pool),
            trust_events,
        ));
        tokio::spawn(reap_idle(Arc::clone(&shared), Arc::clone(&pool)));

        tracing::info!(device = %id.short(), lan_discovery, relay = relay_enabled, "Baton is listening");
        Ok(Node {
            shared,
            endpoint,
            router,
            pool,
            lan_discovery,
            offer_ttl_secs: config.pairing.offer_ttl_secs,
            max_clipboard_bytes: config.handoff.max_clipboard_bytes,
        })
    }
}

/// A device taking part in Baton: reachable by its paired devices, able to pair
/// with new ones, and able to send them things.
///
/// Share a node between tasks with `Arc<Node>`. Dropping the last handle stops it;
/// [`shutdown`](Self::shutdown) does so politely, telling connected devices first.
pub struct Node {
    shared: Arc<Shared>,
    endpoint: Endpoint,
    router: Router,
    pool: Arc<Pool>,
    lan_discovery: bool,
    offer_ttl_secs: u64,
    max_clipboard_bytes: usize,
}

impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Node")
            .field("device_id", &self.shared.id)
            .finish_non_exhaustive()
    }
}

/// How a paired device is reachable right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PingReport {
    /// Round trip time of the ping itself, not counting any time spent connecting.
    pub rtt: Duration,
    /// The route the traffic took.
    pub path: PathKind,
}

/// Where this node can be reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeStatus {
    /// This device.
    pub device_id: DeviceId,
    /// Addresses on this machine's networks that other devices can try directly.
    pub direct_addrs: Vec<SocketAddr>,
    /// The relay other devices can reach this one through, once it is connected to one.
    pub relay: Option<String>,
    /// Whether this node is announcing itself and looking for devices on the local network.
    pub lan_discovery: bool,
}

/// The answer to a request, with the connection it came on.
struct Exchange {
    response: Response,
    connection: iroh::endpoint::Connection,
    rtt: Duration,
}

impl Node {
    /// Starts setting up a node. See [`NodeBuilder`].
    pub fn builder(identity: &Identity, trust: TrustStore, config: Config) -> NodeBuilder {
        NodeBuilder::new(identity, trust, config)
    }

    /// This device's identity.
    pub fn device_id(&self) -> DeviceId {
        self.shared.id
    }

    /// The list of paired devices this node enforces.
    pub fn trust(&self) -> &TrustStore {
        &self.shared.trust
    }

    /// Where this node can be reached right now.
    pub fn status(&self) -> NodeStatus {
        let addr = self.endpoint.addr();
        NodeStatus {
            device_id: self.shared.id,
            direct_addrs: addr.ip_addrs().copied().collect(),
            relay: addr.relay_urls().next().map(ToString::to_string),
            lan_discovery: self.lan_discovery,
        }
    }

    /// How many session connections are open with `peer` right now, in either direction.
    pub fn open_connections(&self, peer: &DeviceId) -> usize {
        self.shared
            .registry
            .open_counts()
            .into_iter()
            .find(|(id, _)| id == peer)
            .map_or(0, |(_, count)| count)
    }

    /// The paired devices that have a session connection to this node open right now.
    pub fn connected_peers(&self) -> Vec<DeviceId> {
        self.shared
            .registry
            .open_counts()
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    /// Opens an invitation to pair and returns it.
    ///
    /// The returned [`PairOffer`] carries the ticket to show on this device's
    /// screen. When a device redeems it, `approver` is asked whether to accept;
    /// only a yes pairs. An earlier offer that is still open is withdrawn.
    ///
    /// # Errors
    ///
    /// If the ticket cannot be made.
    pub async fn offer_pairing(&self, approver: impl PairApprover) -> Result<PairOffer> {
        if self.shared.relay_enabled {
            // A ticket that names the relay works from other networks, so give the
            // endpoint a moment to reach it. Without one the ticket still works nearby.
            let _ = tokio::time::timeout(RELAY_WAIT, self.endpoint.online()).await;
        }
        let (relay, addrs) = ticket_hints(&self.endpoint.addr());
        let secret = PairSecret::from_bytes(random_bytes()?);
        let lifetime = Duration::from_secs(self.offer_ttl_secs);
        // The ticket tells the joiner a wall-clock time, but this device holds the offer
        // open by a clock that nobody can set: a wall clock that jumps while the offer is
        // showing must neither stretch it nor cut it short.
        let expires_at = self
            .shared
            .clock
            .now_unix()
            .saturating_add(lifetime.as_secs());
        let deadline = self.shared.clock.monotonic().saturating_add(lifetime);
        let ticket = PairTicket {
            offerer: self.shared.id,
            secret: secret.clone(),
            expires_at,
            relay,
            addrs,
        };
        // Catch an unusable ticket now, not when somebody tries to show it.
        let text = ticket
            .to_text()
            .map_err(|e| Error::Internal(e.to_string()))?;

        let started = self
            .shared
            .offers
            .open(secret, deadline, Arc::new(approver));
        tokio::spawn(end_offer_when_due(
            Arc::clone(&self.shared),
            started.id,
            deadline,
            started.cancel,
        ));
        Ok(PairOffer {
            ticket,
            text,
            id: started.id,
            shared: Arc::clone(&self.shared),
            done: started.done,
        })
    }

    /// Redeems a ticket made by another device and, if its owner accepts, pairs with it.
    ///
    /// `show_code` is called once, as soon as the secure channel to the other device
    /// is up and before it is asked, with the code that this session produces. Show it
    /// to the person while this call waits: the other device shows its own, and the
    /// person approves there only if the two are the same. That comparison is what tells
    /// a request from the device in their hand from one made by a stranger who copied
    /// the ticket. Say which device the person is pairing with as well; the ticket holds
    /// its key, `ticket.offerer.fingerprint()`. The call can wait as long as the person
    /// on the other device takes to answer.
    ///
    /// # Errors
    ///
    /// If the ticket ran out long ago or was made by this device, the other device
    /// cannot be reached, or it declines.
    pub async fn join_pairing(
        &self,
        ticket: &PairTicket,
        show_code: impl FnOnce(ConfirmationCode),
    ) -> Result<PairedDevice> {
        pairing::join(&self.endpoint, &self.shared, ticket, show_code).await
    }

    /// Checks that a paired device is reachable, and how.
    ///
    /// # Errors
    ///
    /// If the device is not paired or cannot be reached.
    pub async fn ping(&self, peer: &DeviceId) -> Result<PingReport> {
        let nonce = u64::from_be_bytes(random_bytes()?);
        let exchange = self
            .exchange(peer, &Request::Ping(Ping { nonce }), None)
            .await?;
        match exchange.response {
            Response::Pong(pong) if pong.nonce == nonce => Ok(PingReport {
                rtt: exchange.rtt,
                path: selected_path(&exchange.connection).0,
            }),
            _ => Err(Error::protocol("the answer to a ping was not that ping's")),
        }
    }

    /// Hands `item` to a paired device and reports what it did with it.
    ///
    /// A connection that has gone stale since the last use is replaced and the
    /// item sent again under the same id, which the receiver recognises, so an item
    /// is never handled twice.
    ///
    /// # Errors
    ///
    /// If the device is not paired, cannot be reached, does not accept this kind of
    /// item, or the item breaks a limit. A device that is reached but chooses not to
    /// take the item is not an error: the [`Outcome`] says so.
    pub async fn send(&self, peer: &DeviceId, item: Item) -> Result<Outcome> {
        if let Item::ClipboardText(clip) = &item
            && clip.text.len() > self.max_clipboard_bytes
        {
            return Err(Error::BadItem(format!(
                "the text is {} bytes, more than the {} allowed by the settings",
                clip.text.len(),
                self.max_clipboard_bytes
            )));
        }
        let id = ItemId::from_bytes(random_bytes()?);
        let capability = item.required_capability();
        let request = Request::Handoff(Handoff { id, item });
        request
            .validate()
            .map_err(|e| Error::BadItem(e.to_string()))?;

        match self
            .exchange(peer, &request, Some(capability))
            .await?
            .response
        {
            Response::Handoff(reply) if reply.id == id => Ok(reply.outcome),
            _ => Err(Error::protocol(
                "the answer to a handoff was not for that handoff",
            )),
        }
    }

    /// The connection that requests to `peer` go on, made if there is none.
    #[cfg(test)]
    pub(crate) async fn pooled_connection(
        &self,
        peer: &DeviceId,
    ) -> Result<iroh::endpoint::Connection> {
        let paired = self.shared.trust.get(peer).ok_or(Error::NotPaired(*peer))?;
        let (session, _) = self
            .pool
            .session(&self.endpoint, &self.shared, &paired)
            .await?;
        Ok(session.connection)
    }

    /// Sends one request to a paired device, connecting first if needed.
    ///
    /// If a connection from an earlier call turns out to have died, one retry is
    /// made on a new connection.
    async fn exchange(
        &self,
        peer: &DeviceId,
        request: &Request,
        capability: Option<&'static str>,
    ) -> Result<Exchange> {
        let paired = self.shared.trust.get(peer).ok_or(Error::NotPaired(*peer))?;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let (session, reused) = self
                .pool
                .session(&self.endpoint, &self.shared, &paired)
                .await?;
            if let Some(capability) = capability
                && !session.peer.supports(capability)
            {
                return Err(Error::Unsupported(capability.to_owned()));
            }
            let started = Instant::now();
            match call(
                &session.connection,
                request,
                self.shared.tuning.reply_deadline,
            )
            .await
            {
                Ok(response) => {
                    return Ok(Exchange {
                        response,
                        connection: session.connection,
                        rtt: started.elapsed(),
                    });
                }
                // A connection that sat idle may have been closed by the other side
                // or by the network; a fresh one is worth a try. A brand new
                // connection that fails is a real failure.
                Err(e) if reused && attempt == 1 && worth_retrying(&e) => {
                    tracing::debug!(error = %e, "retrying on a new connection");
                    session
                        .connection
                        .close(VarInt::from_u32(close_code::NORMAL), b"stale");
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Stops the node: tells connected devices, withdraws any pairing offer and
    /// closes the network endpoint. Calling it again does nothing.
    pub async fn shutdown(&self) {
        self.shared.stop.cancel();
        self.shared.offers.end_any(OfferEnd::Cancelled);
        self.shared
            .registry
            .close_everything(close_code::INTERNAL, b"shutting down");
        if let Err(e) = self.router.shutdown().await {
            tracing::warn!(error = %e, "the network task did not stop cleanly");
        }
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        // Background tasks hold their own handles to the shared state, so tell them to stop.
        self.shared.stop.cancel();
    }
}

fn relay_mode(network: &NetworkConfig) -> Result<RelayMode> {
    Ok(match network.relay {
        config::RelayMode::Default => RelayMode::Default,
        config::RelayMode::Disabled => RelayMode::Disabled,
        config::RelayMode::Custom => {
            RelayMode::custom(network.parsed_relay_urls()?.into_iter().map(RelayUrl::from))
        }
    })
}

fn portmapper_config(network: &NetworkConfig) -> PortmapperConfig {
    if network.port_mapping {
        PortmapperConfig::default()
    } else {
        PortmapperConfig::Disabled
    }
}

/// Announces this device on the local network and looks for others there. A
/// machine without multicast still works, just without this.
fn start_lan_discovery(endpoint: &Endpoint) -> bool {
    let mdns = match MdnsAddressLookup::builder()
        .service_name(MDNS_SERVICE)
        .build(endpoint.id())
    {
        Ok(mdns) => mdns,
        Err(e) => {
            tracing::warn!(error = %e, "local network discovery could not start");
            return false;
        }
    };
    match endpoint.address_lookup() {
        Ok(services) => {
            services.add(mdns);
            true
        }
        Err(e) => {
            tracing::warn!(error = %e, "local network discovery could not be added");
            false
        }
    }
}

/// The relay and the direct addresses worth putting in a ticket.
fn ticket_hints(addr: &EndpointAddr) -> (Option<String>, Vec<SocketAddr>) {
    let relay = addr
        .relay_urls()
        .next()
        .map(ToString::to_string)
        .filter(|relay| {
            relay.len() <= MAX_RELAY_BYTES && relay.bytes().all(|b| b.is_ascii_graphic())
        });

    let mut addrs: Vec<SocketAddr> = addr
        .ip_addrs()
        .copied()
        .filter(|addr| is_dialable(addr.ip()))
        .collect();
    // Another machine cannot use this machine's loopback address. It is all there
    // is, though, when two nodes on one machine pair, which is what tests do.
    if addrs.iter().any(|addr| !addr.ip().is_loopback()) {
        addrs.retain(|addr| !addr.ip().is_loopback());
    }
    // Addresses on the local network are the likeliest to work, so they go first.
    addrs.sort_by_key(|addr| !is_local(addr.ip()));
    addrs.truncate(MAX_ADDRS);
    (relay, addrs)
}

/// Ends an offer when its time is up, so that waiting on it returns.
///
/// An offer that a request is with the person for is left alone, and looked at again
/// until it has been answered, timed out or withdrawn: the person is not interrupted
/// in the middle of answering.
async fn end_offer_when_due(
    shared: Arc<Shared>,
    id: u64,
    deadline: Duration,
    ended: CancellationToken,
) {
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    loop {
        tokio::select! {
            biased;
            () = ended.cancelled() => return,
            () = shared.stop.cancelled() => return,
            _ = tick.tick() => {}
        }
        if shared.clock.monotonic() >= deadline && shared.offers.expire(id) {
            return;
        }
    }
}

/// Closes connections to a device the moment it is unpaired.
async fn watch_trust(
    shared: Arc<Shared>,
    pool: Arc<Pool>,
    mut events: broadcast::Receiver<TrustEvent>,
) {
    loop {
        tokio::select! {
            biased;
            () = shared.stop.cancelled() => return,
            event = events.recv() => match event {
                Ok(TrustEvent::Removed(peer)) => {
                    pool.forget(&peer);
                    shared.intake.forget(&peer);
                    let closed = shared.registry.close_peer(&peer, close_code::NOT_PAIRED, b"unpaired");
                    tracing::info!(peer = %peer.short(), closed, "a device was unpaired");
                }
                Ok(TrustEvent::Added(_) | TrustEvent::Updated(_)) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Some events were missed, so compare with the list itself.
                    let trust = shared.trust.clone();
                    shared.registry.close_unless(
                        |peer| trust.contains(peer),
                        close_code::NOT_PAIRED,
                        b"unpaired",
                    );
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }
}

/// Closes connections to paired devices that have not been used for a while, so
/// that a phone is not kept awake by connections nobody needs.
async fn reap_idle(shared: Arc<Shared>, pool: Arc<Pool>) {
    let idle = shared.tuning.idle_connection;
    let every = (idle / 4).clamp(Duration::from_millis(100), Duration::from_secs(30));
    let mut tick = tokio::time::interval(every);
    loop {
        tokio::select! {
            biased;
            () = shared.stop.cancelled() => return,
            _ = tick.tick() => pool.reap(idle),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(text: &str) -> SocketAddr {
        text.parse().unwrap()
    }

    #[test]
    fn tickets_prefer_local_addresses_and_skip_loopback_when_there_is_better() {
        let endpoint_id = SecretKey::from_bytes(&[1; 32]).public();
        let mut address = EndpointAddr::new(endpoint_id);
        for a in [
            "127.0.0.1:1",
            "8.8.8.8:2",
            "192.168.1.5:3",
            "[fe80::1]:4",
            "[fd00::5]:5",
        ] {
            address = address.with_ip_addr(addr(a));
        }
        let (relay, addrs) = ticket_hints(&address);
        assert_eq!(relay, None);
        assert_eq!(
            addrs,
            vec![
                addr("192.168.1.5:3"),
                addr("[fd00::5]:5"),
                addr("8.8.8.8:2")
            ]
        );
    }

    #[test]
    fn loopback_is_kept_when_it_is_all_there_is() {
        let endpoint_id = SecretKey::from_bytes(&[1; 32]).public();
        let address = EndpointAddr::new(endpoint_id).with_ip_addr(addr("127.0.0.1:9"));
        assert_eq!(ticket_hints(&address).1, vec![addr("127.0.0.1:9")]);
    }

    #[test]
    fn tickets_name_the_relay_and_cap_the_addresses() {
        let endpoint_id = SecretKey::from_bytes(&[1; 32]).public();
        let mut address = EndpointAddr::new(endpoint_id)
            .with_relay_url("https://relay.example./".parse().unwrap());
        for i in 0..40u16 {
            address = address.with_ip_addr(addr(&format!("10.0.0.{}:{}", i + 1, 1000 + i)));
        }
        let (relay, addrs) = ticket_hints(&address);
        assert!(relay.is_some_and(|r| r.starts_with("https://relay.example")));
        assert_eq!(addrs.len(), MAX_ADDRS);
    }

    #[test]
    fn default_time_limits_leave_room_for_a_person() {
        let tuning = Tuning::default();
        assert!(tuning.pair_connection_limit() > tuning.approval_timeout);
        assert!(tuning.join_reply_deadline() > tuning.approval_timeout);
        assert!(tuning.reply_deadline > tuning.request_deadline);
    }
}
