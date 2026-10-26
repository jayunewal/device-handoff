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
    ALPN_PAIR, ALPN_SESSION, DeviceId, DeviceInfo, ItemId, Platform, TransferId, close_code,
    id::SessionId,
    message::{Handoff, Item, Message, Outcome, Ping, Request, Response, capability},
    pair::{ConfirmationCode, PairSecret},
    stream::{Start, StreamReply, StreamRequest},
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
use crate::uni::Sink;
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
    streaming::{
        CONTROL_STREAMS, FRAME_STREAMS, Offered, Reservation, SETTLE, SOUND_STREAMS, ShareOutcome,
        Sharer, SourceEvents, StartHandler, StartOutcome, StartSlot, Starter, Streaming, Takes,
        Viewer, Viewing, Wanted, WhyNot, WindowHandler, WindowShare, Withdraw, ask,
    },
    transfer::{
        incoming::{FINISH_WAIT, Incoming, LANE_STREAMS, TransferHandler},
        lanes::LaneSink,
        receive::STALL,
        send::{self, Holds, Ledger, NodeWire, Pace, Policy, Selection, Sending, Wire},
    },
    transport,
    trust::{PairedDevice, TrustEvent, TrustStore},
    uni::{Kind, Sinks, Uni},
    util::random_bytes,
};

/// The name devices use to find each other on the local network.
const MDNS_SERVICE: &str = "baton";

/// How long to wait for the relay before making a ticket without it.
const RELAY_WAIT: Duration = Duration::from_secs(3);

/// The table of the transfers that other devices offer, when the application takes transfers. Its
/// waits are the tuning's, so that a test can shorten them.
fn incoming_table(
    transfers: Option<Arc<dyn TransferHandler>>,
    intake: &Intake,
    tuning: &Tuning,
) -> Option<Incoming> {
    transfers.map(|handler| {
        Incoming::with_waits(
            handler,
            intake.item_charge(),
            tuning.finish_wait,
            tuning.stall,
        )
    })
}

/// Installs what takes the unidirectional streams of the kinds that this node takes part in: the
/// lanes of a transfer when it receives files, the frames of window streaming when it shows
/// windows, the packets of sound when it plays them, and the control streams of the sessions it
/// has, as a viewer or as a source. Returns them with the windows of the node.
fn take_streams(
    sinks: Sinks,
    incoming: Option<&Incoming>,
    viewer: Option<Viewer>,
    sharer: Option<Sharer>,
    starts: Option<Arc<dyn StartHandler>>,
    intake: &Intake,
    tuning: &Tuning,
) -> (Sinks, Arc<Streaming>) {
    let sinks = match incoming {
        Some(incoming) => sinks.install(
            Kind::Lane,
            LANE_STREAMS,
            Arc::new(LaneSink::new(incoming.clone(), tuning.request_deadline)),
        ),
        None => sinks,
    };
    let sharer = sharer.map(|sharer| Sharer {
        starter: starts
            .map(|handler| Starter::new(handler, intake.item_charge(), tuning.start_prepare)),
        ..sharer
    });
    let streaming = Streaming::taking_part(viewer, sharer);
    let sinks = if streaming.views() {
        sinks.install(Kind::Video, FRAME_STREAMS, streaming.clone())
    } else {
        sinks
    };
    let sinks = if streaming.hears() {
        sinks.install(Kind::Sound, SOUND_STREAMS, streaming.clone())
    } else {
        sinks
    };
    // A connection can carry a session of each side, and each has a control stream from the other
    // end.
    let controls = u32::from(streaming.views()) + u32::from(streaming.shares());
    let sinks = if controls > 0 {
        sinks.install(Kind::Control, controls * CONTROL_STREAMS, streaming.clone())
    } else {
        sinks
    };
    (sinks, streaming)
}

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
    /// How long a person has to approve a pairing request, and to answer the offer of a window.
    pub approval_timeout: Duration,
    /// How long a device that was asked to start a session has to get ready before it must
    /// answer. The device that asked waits [`reply_deadline`](Self::reply_deadline) for the
    /// answer, so this is shorter.
    pub start_prepare: Duration,
    /// How long a device that asked another to start a session waits for the offer, once the
    /// other has said that it will make one.
    pub start_wait: Duration,
    /// How long a refused pairing request is held before the refusal is sent.
    pub invalid_delay: Duration,
    /// How long an unused connection to a paired device is kept open.
    pub idle_connection: Duration,
    /// How long a `Finish` of a transfer waits for the files to settle. When they have not by
    /// then, the request ends without an answer (a stop with 408) and the sender asks again.
    pub finish_wait: Duration,
    /// How long a transfer that is being received may move no byte before its attempt is
    /// stopped (section 8, Time). The sender may try again, and the files that were received stay.
    pub stall: Duration,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            request_deadline: Duration::from_secs(10),
            reply_deadline: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(20),
            approval_timeout: Duration::from_mins(2),
            start_prepare: Duration::from_secs(20),
            start_wait: Duration::from_secs(30),
            invalid_delay: Duration::from_millis(250),
            idle_connection: Duration::from_mins(2),
            finish_wait: FINISH_WAIT,
            stall: STALL,
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

    /// How long the source of a window waits for the answer to an offer: the approval plus the
    /// network.
    pub(crate) fn offer_reply_deadline(&self) -> Duration {
        self.approval_timeout + Duration::from_secs(30)
    }
}

/// Sets up a [`Node`].
pub struct NodeBuilder {
    secret: SecretKey,
    trust: TrustStore,
    config: Config,
    handler: Arc<dyn HandoffHandler>,
    /// What the application says about the transfers that other devices offer, if it takes them.
    transfers: Option<Arc<dyn TransferHandler>>,
    /// What the application says about the windows that other devices offer, if it shows them.
    windows: Option<Arc<dyn WindowHandler>>,
    /// Whether this device shows its own windows to others, and takes input if it does.
    sharing: Option<Sharer>,
    /// Whether this device sends the sound of the screens it shares.
    sends_sound: bool,
    /// Whether this device plays the sound of the screens it is shown.
    plays_sound: bool,
    /// What the application does when a paired device asks this one to start a session, if it
    /// starts them.
    starts: Option<Arc<dyn StartHandler>>,
    /// What other devices may be holding for this one, if the application keeps it.
    holds: Option<Holds>,
    clock: Arc<dyn Clock>,
    tuning: Tuning,
    /// What takes the unidirectional streams that paired devices open, besides the lanes of the
    /// transfers, which `bind` installs when there is a handler for them, and the frames and
    /// control streams of window streaming, which it installs for a device that shows windows.
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
            transfers: None,
            windows: None,
            sharing: None,
            sends_sound: false,
            plays_sound: false,
            starts: None,
            holds: None,
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

    /// Has this device take the files and folders that other devices offer, asking `handler` about
    /// each offer and where to put what is received.
    ///
    /// Without one, a device takes no files and says nothing about them. With one, it advertises
    /// `transfer.files` for as long as [`TransferHandler::destination`] has a folder to give.
    #[must_use]
    pub fn transfer_handler(mut self, handler: impl TransferHandler) -> Self {
        self.transfers = Some(Arc::new(handler));
        self
    }

    /// Has this device show the windows that paired devices offer, asking `handler` about each
    /// offer and giving it the pictures to show.
    ///
    /// Without one, a device shows no window and declines every offer as unsupported. With one,
    /// it advertises `stream.view`.
    #[must_use]
    pub fn window_handler(mut self, handler: impl WindowHandler) -> Self {
        self.windows = Some(Arc::new(handler));
        self
    }

    /// Has this device offer its windows to paired devices, with [`Node::share`].
    ///
    /// Without it, a device offers none and says nothing about it. With it, the device advertises
    /// `stream.share`. With `control` it also takes input from a viewer in a session where the
    /// person at this device granted control, and advertises `stream.control`.
    #[must_use]
    pub fn share_windows(mut self, control: bool) -> Self {
        self.sharing = Some(Sharer {
            control,
            starter: None,
            sound: false,
        });
        self
    }

    /// Has this device send the sound that it plays along with a whole display that it shares,
    /// when the viewer asks to hear it ([`SourceEvents::hear`]).
    ///
    /// It needs [`share_windows`](Self::share_windows), and [`bind`](Self::bind) fails without it.
    /// Without it, a device sends no sound and says nothing about it. With it, the device
    /// advertises `stream.sound`. It sends sound only to a device that advertised `stream.hear`.
    #[must_use]
    pub fn share_sound(mut self) -> Self {
        self.sends_sound = true;
        self
    }

    /// Has this device play the sound of a screen that a paired device shares with it, when the
    /// application asks for it ([`ViewHandle::hear`](crate::streaming::ViewHandle::hear)) and gave
    /// the session a [`Speaker`](crate::streaming::Speaker) ([`Viewing::with_speaker`]).
    ///
    /// It needs [`window_handler`](Self::window_handler), and [`bind`](Self::bind) fails without
    /// it. Without it, a device plays no sound and says nothing about it. With it, the device
    /// advertises `stream.hear`.
    #[must_use]
    pub fn play_sound(mut self) -> Self {
        self.plays_sound = true;
        self
    }

    /// Has this device start a session when a paired device that the person here allowed asks it
    /// to, asking `handler` to get ready each time. The person is not asked: they decided when they
    /// allowed the device ([`StartPolicy`](crate::StartPolicy)).
    ///
    /// It needs [`share_windows`](Self::share_windows), and [`bind`](Self::bind) fails without it.
    /// Without a handler, a device starts no session and says nothing about it. With one, it
    /// advertises `stream.start`.
    #[must_use]
    pub fn start_handler(mut self, handler: impl StartHandler) -> Self {
        self.starts = Some(Arc::new(handler));
        self
    }

    /// Has this device remember the transfers of files that it offers to other devices, in `holds`,
    /// so that a device that is left holding one is told to let go of it, also by a run of the
    /// program that starts later. A transfer that is not ended properly, because the program is
    /// closed or the network is down, would otherwise stay at the receiver for an hour, and count
    /// against the transfers that this device may have going there.
    ///
    /// Without it, a device remembers them for as long as the program runs. See [`Holds`].
    #[must_use]
    pub fn holds(mut self, holds: Holds) -> Self {
        self.holds = Some(holds);
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

    /// Checks the settings, and that each part of the device that needs another part has it.
    fn check(&self) -> Result<()> {
        self.config.validate()?;
        let missing = if self.starts.is_some() && self.sharing.is_none() {
            "a handler for starting sessions needs the device to share its windows"
        } else if self.sends_sound && self.sharing.is_none() {
            "sending sound needs the device to share its windows"
        } else if self.plays_sound && self.windows.is_none() {
            "playing sound needs the device to show the windows of others"
        } else {
            return Ok(());
        };
        Err(Error::Config(missing.to_owned()))
    }

    /// Starts the node: binds the network endpoint and begins serving.
    ///
    /// # Errors
    ///
    /// If the settings are invalid or the endpoint cannot be bound.
    pub async fn bind(self) -> Result<Node> {
        self.check()?;
        let Self {
            secret,
            trust,
            config,
            handler,
            transfers,
            windows,
            sharing,
            sends_sound,
            plays_sound,
            starts,
            holds,
            clock,
            tuning,
            sinks,
            tweak,
            #[cfg(any(test, feature = "test-support"))]
            before_registering,
        } = self;
        let holds = holds.unwrap_or_else(Holds::memory);

        let id = DeviceId::from_bytes(*secret.public().as_bytes());
        let relay_enabled = config.network.relay != config::RelayMode::Disabled;
        let registry = Arc::new(ConnectionRegistry::default());
        let stop = CancellationToken::new();
        let intake = Intake::new(&config.handoff);
        let incoming = incoming_table(transfers, &intake, &tuning);
        let viewer = windows.map(|handler| {
            Viewer::new(handler, intake.item_charge(), tuning.approval_timeout).hearing(plays_sound)
        });
        let sharing = sharing.map(|sharer| Sharer {
            sound: sends_sound,
            ..sharer
        });
        let (sinks, streaming) = take_streams(
            sinks,
            incoming.as_ref(),
            viewer,
            sharing,
            starts,
            &intake,
            &tuning,
        );
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
            intake,
            incoming,
            streaming,
            uni: Arc::clone(&uni),
        });

        let gate = Gate::new(trust.clone(), registry, uni.quic_limit());
        #[cfg(any(test, feature = "test-support"))]
        let gate = match before_registering {
            Some(hook) => gate.before_registering(hook),
            None => gate,
        };
        let endpoint = bind_endpoint(secret, &config, gate, tweak).await?;
        let lan_discovery = config.network.lan_discovery && start_lan_discovery(&endpoint);

        let (router, pool) = serve(&shared, &endpoint, &holds);

        tracing::info!(device = %id.short(), lan_discovery, relay = relay_enabled, "Baton is listening");
        Ok(Node {
            shared,
            endpoint,
            router,
            pool,
            lan_discovery,
            offer_ttl_secs: config.pairing.offer_ttl_secs,
            max_clipboard_bytes: config.handoff.max_clipboard_bytes,
            holds,
        })
    }
}

/// Starts serving: the router that takes the connections of both protocols, and the tasks that act
/// on a device being unpaired, close the connections that went idle and tell the devices that hold
/// transfers for this one to let go of them. Returns the router and the pool of outgoing
/// connections.
fn serve(shared: &Arc<Shared>, endpoint: &Endpoint, holds: &Holds) -> (Router, Arc<Pool>) {
    // Subscribe before the first connection can arrive, so no unpairing is missed.
    let trust_events = shared.trust.subscribe();
    let admission = Arc::new(Admission::new(shared.trust.clone()));
    let router = Router::builder(endpoint.clone())
        .accept(
            ALPN_SESSION,
            SessionServer {
                shared: Arc::clone(shared),
            },
        )
        .accept(
            ALPN_PAIR,
            PairServer {
                shared: Arc::clone(shared),
            },
        )
        .incoming_filter(Arc::new(move |incoming| admission.judge(incoming)))
        .spawn();

    let pool = Arc::new(Pool::default());
    tokio::spawn(watch_trust(
        Arc::clone(shared),
        Arc::clone(&pool),
        trust_events,
    ));
    tokio::spawn(reap_idle(Arc::clone(shared), Arc::clone(&pool)));
    tokio::spawn(start_release(shared, endpoint, holds));
    (router, pool)
}

/// Tells the devices that may hold transfers that this device left behind to let go of them, from
/// the start of the node until it stops. See [`send::release_holds`].
fn start_release(
    shared: &Arc<Shared>,
    endpoint: &Endpoint,
    holds: &Holds,
) -> impl std::future::Future<Output = ()> + Send + 'static {
    let (wires_shared, wires_endpoint) = (Arc::clone(shared), endpoint.clone());
    let trust = shared.trust.clone();
    send::release_holds(
        holds.clone(),
        move |peer| {
            Arc::new(NodeWire::new(
                wires_endpoint.clone(),
                Arc::clone(&wires_shared),
                peer,
            ))
        },
        Arc::new(move |peer: &DeviceId| trust.contains(peer)),
        Arc::clone(&shared.clock),
        Pace::default(),
        shared.stop.clone(),
    )
}

/// Binds the network endpoint of a node, behind `gate`, which admits the paired devices only.
async fn bind_endpoint(
    secret: SecretKey,
    config: &Config,
    gate: Gate,
    tweak: Option<Box<dyn FnOnce(Builder) -> Builder + Send>>,
) -> Result<Endpoint> {
    let mut builder = Endpoint::builder(presets::Minimal)
        .secret_key(secret)
        .relay_mode(relay_mode(&config.network)?)
        .hooks(gate)
        .transport_config(transport::config())
        .portmapper_config(portmapper_config(&config.network));
    if config.network.dns_discovery && config.network.relay != config::RelayMode::Disabled {
        builder = builder
            .address_lookup(PkarrPublisher::n0_dns())
            .address_lookup(PkarrResolver::n0_dns())
            .address_lookup(DnsAddressLookup::n0_dns());
    }
    if let Some(tweak) = tweak {
        builder = tweak(builder);
    }
    builder
        .bind()
        .await
        .map_err(|e| Error::network("starting the network endpoint", &e))
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
    /// What other devices may be holding for this one.
    holds: Holds,
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

    /// Starts sending the files and folders of `selection` to a paired device, and returns at once
    /// with the transfer that is going.
    ///
    /// The person at the other end is asked whether to take them, and may decline. The transfer
    /// goes on connections of its own, a new one for each attempt, and is tried again when a
    /// connection goes or the other device stalls, until the patience of the sender runs out.
    /// [`Sending::progress`] shows which of these it is doing, [`Sending::cancel`] gives it up and
    /// [`Sending::outcome`] says how it ended. Dropping the [`Sending`] gives the transfer up, and so
    /// does stopping the node.
    ///
    /// A device that takes no part in transfers declines the offer as unsupported, which is an
    /// [`Outcome::Declined`](send::Outcome::Declined) like a person saying no.
    ///
    /// This must be called from within a Tokio runtime, as the transfer is run as a task of its own.
    ///
    /// # Errors
    ///
    /// If the device is not paired, or a number for the transfer's id cannot be had.
    pub fn send_files(&self, peer: &DeviceId, selection: Selection) -> Result<Sending> {
        self.send_files_with(peer, selection, Policy::default())
    }

    /// [`send_files`](Self::send_files) with other numbers than the usual ones, which the tests of
    /// this crate use to see a transfer give up in less than minutes.
    pub(crate) fn send_files_with(
        &self,
        peer: &DeviceId,
        selection: Selection,
        policy: Policy,
    ) -> Result<Sending> {
        if self.shared.trust.get(peer).is_none() {
            return Err(Error::NotPaired(*peer));
        }
        self.send_over(peer, Arc::new(self.wire_to(peer)), selection, policy)
    }

    /// Starts a transfer to `peer` over `wire`. It is [`wire_to`](Self::wire_to) for a device, except
    /// in the tests of this crate, which put something between the transfer and the connections to
    /// hold the lanes at a moment they choose.
    pub(crate) fn send_over(
        &self,
        peer: &DeviceId,
        wire: Arc<dyn Wire>,
        selection: Selection,
        policy: Policy,
    ) -> Result<Sending> {
        let id = TransferId::from_bytes(random_bytes()?);
        let ledger = Ledger::new(self.holds.clone(), *peer, Arc::clone(&self.shared.clock));
        send::start(
            wire,
            Arc::new(selection),
            policy,
            id,
            &self.shared.stop,
            Some(ledger),
        )
        .map_err(|error| Error::Internal(error.to_string()))
    }

    /// The connections that a transfer to `peer` goes over: a new one for each attempt.
    pub(crate) fn wire_to(&self, peer: &DeviceId) -> NodeWire {
        NodeWire::new(self.endpoint.clone(), Arc::clone(&self.shared), *peer)
    }

    /// Closes every connection to `peer`, as a network that fails would, and says how many were
    /// open.
    #[cfg(test)]
    pub(crate) fn drop_connections(&self, peer: &DeviceId) -> usize {
        self.shared
            .registry
            .close_peer(peer, close_code::INTERNAL, b"a test")
    }

    /// Offers a window of this device to a paired device and, if the person there accepts it,
    /// starts sending its picture.
    ///
    /// The other device decides once, and a person may be asked, so the answer can take as long
    /// as a person may take (two minutes and the network). Dropping the returned future before it
    /// is done withdraws the offer, and the other device takes the question back from its
    /// person. On a yes `choose` is shown what the other device can take, and says what picture
    /// the platform makes within it, or `None` if it can make none, which ends the session at
    /// once ([`ShareOutcome::Unmatched`]). The platform is then told what the viewer reports and
    /// wants through `events`, and asks the [`SourceHandle`](crate::streaming::SourceHandle) in
    /// the outcome, picture by picture, whether to encode.
    ///
    /// A device has one session as a source at a time. The connection to the other device is kept
    /// open for as long as the session runs.
    ///
    /// A window offered straight after another was given up, or after a session of this device
    /// ended, is offered once the other device has answered the `Stop` that this device sent for
    /// the first (three seconds at most), so the other device does not turn it away as busy while
    /// it still holds the place. One that does not answer in time may still do so
    /// ([`ShareOutcome::Declined`] with `RateLimited`), and offering again a moment later works.
    ///
    /// # Errors
    ///
    /// If this device was not set up to show its windows, or not to take input when `window`
    /// grants control, or `window` breaks a limit ([`Error::BadItem`]); if the device is not
    /// paired, cannot be reached, does not advertise `stream.view` or breaks the protocol; if a
    /// window is being offered or shared already ([`Error::Busy`]); or if the device is unpaired
    /// or this node stops while the offer waits. A device that is reached but does not take the
    /// window is not an error: the [`ShareOutcome`] says so.
    pub async fn share(
        &self,
        peer: &DeviceId,
        window: WindowShare,
        events: impl SourceEvents,
        choose: impl FnOnce(&Takes) -> Option<baton_stream::control::Config> + Send,
    ) -> Result<ShareOutcome> {
        let paired = self.shared.trust.get(peer).ok_or(Error::NotPaired(*peer))?;
        let streaming = &self.shared.streaming;
        streaming.can_offer(&window)?;
        // A `Stop` that this device sent to the same device a moment ago, for an offer that was
        // given up or a session that was ended, is answered first: until then the other device
        // may still hold the place of what was stopped, and refuse this offer as busy.
        streaming.settled(&paired.id, SETTLE).await;
        let reservation = streaming.reserve(paired.id)?;
        self.offer_window(reservation, &paired, None, window, events, choose)
            .await
    }

    /// Offers a window of this device to the paired device that asked for one
    /// ([`StartHandler`]), and, if it accepts, starts sending its picture. The device accepts at
    /// once, since its person asked when they started it, and nobody is asked.
    ///
    /// This is [`share`](Self::share) for the place and the session id that `slot` holds: the
    /// offer is made under the id that the device chose, and the place was taken when it asked,
    /// so nothing else can have been offered meanwhile. `window` may grant control only if the
    /// slot does ([`StartSlot::control`]); it may grant less.
    ///
    /// A device that takes its asking back while this waits ends it with
    /// [`ShareOutcome::Cancelled`], and nothing is shared.
    ///
    /// # Errors
    ///
    /// The same as [`share`](Self::share), and [`Error::Config`] when `window` grants control
    /// that the slot does not. The place is let go of when this fails.
    pub async fn share_asked(
        &self,
        slot: StartSlot,
        window: WindowShare,
        events: impl SourceEvents,
        choose: impl FnOnce(&Takes) -> Option<baton_stream::control::Config> + Send,
    ) -> Result<ShareOutcome> {
        let StartSlot {
            reservation,
            session,
            peer,
            control,
        } = slot;
        let paired = self.shared.trust.get(&peer).ok_or(Error::NotPaired(peer))?;
        self.shared.streaming.can_offer(&window)?;
        if window.control() && !control {
            return Err(Error::Config(
                "control of the window was granted, but the device that asked is not allowed it"
                    .to_owned(),
            ));
        }
        self.offer_window(reservation, &paired, Some(session), window, events, choose)
            .await
    }

    /// Makes the offer of `window` to `paired`, for the place that `reservation` holds, and
    /// starts the session if the answer is a yes. `asked` is the session id that the device chose,
    /// if it asked for the offer.
    async fn offer_window(
        &self,
        reservation: Reservation,
        paired: &PairedDevice,
        asked: Option<SessionId>,
        window: WindowShare,
        events: impl SourceEvents,
        choose: impl FnOnce(&Takes) -> Option<baton_stream::control::Config> + Send,
    ) -> Result<ShareOutcome> {
        let streaming = &self.shared.streaming;
        let wait = self.shared.tuning.offer_reply_deadline();
        let mut attempt = 0;
        let (session, hold, id, reply) = loop {
            attempt += 1;
            let (session, reused) = self
                .pool
                .session(&self.endpoint, &self.shared, paired)
                .await?;
            if !session.peer.supports(capability::STREAM_VIEW) {
                return Err(Error::Unsupported(capability::STREAM_VIEW.to_owned()));
            }
            // The pool sees a request only when it starts, and the person has minutes to answer.
            let hold = session.hold();
            // Nothing is repeated under a session id on a connection, so a second attempt on a
            // new connection has an id of its own, unless the other device chose it: it waits for
            // that one.
            let id = match asked {
                Some(id) => id,
                None => SessionId::from_bytes(random_bytes()?),
            };
            // A device that asked for the offer may have taken its asking back since the place was
            // taken, while the application got ready. Then nothing is offered, and the person
            // there is not asked about an offer that nobody wants: the cancellation is looked at
            // first.
            let offered = tokio::select! {
                biased;
                () = reservation.cancelled() => return self.cancelled_offer(&paired.id, &reservation),
                offered = ask(streaming, paired.id, &session.connection, window.offer(id), wait) => offered,
            };
            match offered {
                Ok(reply) => break (session, hold, id, reply),
                // A connection that sat idle may have been closed. An offer that nobody answered
                // is not repeated: the person there may be deciding.
                Err(e)
                    if reused
                        && attempt == 1
                        && worth_retrying(&e)
                        && !matches!(e, Error::Timeout(_)) =>
                {
                    tracing::debug!(error = %e, "retrying on a new connection");
                    session
                        .connection
                        .close(VarInt::from_u32(close_code::NORMAL), b"stale");
                }
                Err(e) => return Err(self.explain(&paired.id, e)),
            }
        };
        if reservation.is_cancelled() {
            return self.cancelled_offer(&paired.id, &reservation);
        }
        let offered = Offered {
            peer: paired,
            connection: &session.connection,
            hold,
            session: id,
            control: window.control(),
            display: window.is_display(),
            plays: session.peer.supports(capability::STREAM_HEAR),
        };
        streaming.begin(offered, reply, events, choose).await
    }

    /// What came of an offer that waited and was cancelled: the device took its asking back, or
    /// it was unpaired, or this node stopped.
    fn cancelled_offer(&self, peer: &DeviceId, reservation: &Reservation) -> Result<ShareOutcome> {
        if self.shared.stop.is_cancelled() {
            Err(Error::Stopped)
        } else if reservation.is_withdrawn() {
            Ok(ShareOutcome::Cancelled)
        } else {
            Err(Error::NotPaired(*peer))
        }
    }

    /// Asks a paired device to start sharing its screen with this one, and shows it when it does.
    ///
    /// The person at the other device has to have allowed this device beforehand, once, in the
    /// settings for it ([`StartPolicy`](crate::StartPolicy)); nothing sent from here can. If they
    /// did, the other device makes an offer under the id that this call chose, and this device
    /// takes it without asking a person: the person here asked when they started. An offer of any
    /// other kind, from any other device, or under any other id, is not taken that way. `viewing`
    /// is what this device takes and shows the picture with, and `control` whether the person
    /// wishes to control what they see; the offer says what was granted, which is never more.
    ///
    /// The call is done when the session runs ([`StartOutcome::Started`]) or the other device
    /// says no ([`StartOutcome::NotStarted`], with a reason a person can be told). The window
    /// handler is not told of a session started this way: the handle is in the outcome. Dropping
    /// the future before it is done takes the asking back, and the other device gives up the
    /// offer.
    ///
    /// A device shows one window at a time, so this fails while one is shown, or an offer waits
    /// for the person, or another start waits.
    ///
    /// # Errors
    ///
    /// If this device was not set up to show the windows of others ([`Error::Config`]) or what is
    /// wanted is not something to ask for ([`Error::BadItem`]); if the device is not paired,
    /// cannot be reached, does not advertise `stream.start` or breaks the protocol; if a window is
    /// shown or offered already ([`Error::Busy`]); if the device is unpaired or this node stops
    /// while the start waits; or if the other device says that it will make an offer and does not
    /// in [`Tuning::start_wait`], when it is told to give up.
    pub async fn start(
        &self,
        peer: &DeviceId,
        wanted: Wanted,
        control: bool,
        viewing: Viewing,
    ) -> Result<StartOutcome> {
        let paired = self.shared.trust.get(peer).ok_or(Error::NotPaired(*peer))?;
        let streaming = &self.shared.streaming;
        // A `Stop` that this device sent a moment ago may not be answered yet, and the other
        // device may still hold the place of what it stopped.
        streaming.settled(&paired.id, SETTLE).await;
        let session = SessionId::from_bytes(random_bytes()?);
        let mut expecting = streaming.expect_start(paired.id, session, wanted, control, viewing)?;
        let request = Request::Stream(StreamRequest::Start(Start {
            session,
            wanted,
            control,
        }));
        let mut attempt = 0;
        let (mut withdraw, reply) = loop {
            attempt += 1;
            let (via, reused) = self
                .pool
                .session(&self.endpoint, &self.shared, &paired)
                .await?;
            if !via.peer.supports(capability::STREAM_START) {
                return Err(Error::Unsupported(capability::STREAM_START.to_owned()));
            }
            // From here the other device may have the start, so it is told to give it up if this
            // call is dropped or fails.
            let mut withdraw = Withdraw::new(streaming, paired.id, &via.connection, session);
            match call(&via.connection, &request, self.shared.tuning.reply_deadline).await {
                Ok(Response::Stream(reply)) if reply.session() == session => {
                    break (withdraw, reply);
                }
                Ok(_) => {
                    return Err(Error::protocol(
                        "the answer to a start was not for that start",
                    ));
                }
                // A connection that sat idle may have been closed. The start is made again under
                // the same id on a new connection, which the other device answers as it answered
                // the first if that arrived, so there is nothing to take back.
                Err(e) if reused && attempt == 1 && worth_retrying(&e) => {
                    tracing::debug!(error = %e, "retrying on a new connection");
                    withdraw.disarm();
                    via.connection
                        .close(VarInt::from_u32(close_code::NORMAL), b"stale");
                }
                Err(e) => return Err(self.explain(&paired.id, e)),
            }
        };
        match reply {
            StreamReply::Starting(_) => {}
            StreamReply::NotStarted(refusal) => {
                withdraw.disarm();
                return Ok(StartOutcome::NotStarted(refusal.why));
            }
            // A device that takes no part in window streaming answers every request this way.
            StreamReply::Declined(_) => {
                withdraw.disarm();
                return Ok(StartOutcome::NotStarted(WhyNot::Unsupported));
            }
            StreamReply::Accepted(_) | StreamReply::Stopped(_) => {
                return Err(Error::protocol(
                    "the answer to a start was not one to a start",
                ));
            }
        }
        match tokio::time::timeout(self.shared.tuning.start_wait, &mut expecting.offer).await {
            Ok(Ok(Ok(handle))) => {
                withdraw.disarm();
                Ok(StartOutcome::Started(handle))
            }
            // The other device ended it, or did not offer what it said it would.
            Ok(Ok(Err(why))) => {
                withdraw.disarm();
                Ok(StartOutcome::NotStarted(why))
            }
            // The device was unpaired or this node stopped while the start waited.
            Ok(Err(_dropped)) => Err(self.explain(
                &paired.id,
                Error::Internal("the start was dropped while it waited".to_owned()),
            )),
            // `withdraw` is armed, and tells the other device to give up the offer.
            Err(_elapsed) => Err(Error::Timeout(
                "waiting for the other device to offer its screen",
            )),
        }
    }

    /// What a failure while a device was asked really was. This node stopping and the device being
    /// unpaired both close the connection, and the closed connection can be what the call sees
    /// first.
    fn explain(&self, peer: &DeviceId, error: Error) -> Error {
        if self.shared.stop.is_cancelled() {
            Error::Stopped
        } else if self.shared.trust.get(peer).is_none() {
            Error::NotPaired(*peer)
        } else {
            error
        }
    }

    /// The connection that requests to `peer` go on, made if there is none.
    #[cfg(test)]
    pub(crate) async fn pooled_session(&self, peer: &DeviceId) -> Result<crate::session::Session> {
        let paired = self.shared.trust.get(peer).ok_or(Error::NotPaired(*peer))?;
        let (session, _) = self
            .pool
            .session(&self.endpoint, &self.shared, &paired)
            .await?;
        Ok(session)
    }

    /// The window streaming sessions of this node.
    #[cfg(test)]
    pub(crate) fn streaming(&self) -> Arc<Streaming> {
        Arc::clone(&self.shared.streaming)
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
        self.shared.streaming.shutdown();
        if let Some(incoming) = &self.shared.incoming {
            // The lanes ended with the connections, so this does not wait for them.
            incoming.shutdown().await;
        }
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
                    shared.streaming.unpair(&peer);
                    let closed = shared.registry.close_peer(&peer, close_code::NOT_PAIRED, b"unpaired");
                    if let Some(incoming) = shared.incoming.clone() {
                        // The lanes end with the connections that were just closed, and the
                        // answers wait for them, so this is not waited for here.
                        drop(tokio::spawn(async move { incoming.forget(&peer).await }));
                    }
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
                    shared.streaming.unpair_unless(|peer| trust.contains(peer));
                    if let Some(incoming) = shared.incoming.clone() {
                        let trust = shared.trust.clone();
                        drop(tokio::spawn(async move {
                            incoming.forget_unless(|peer| trust.contains(peer)).await;
                        }));
                    }
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

    #[test]
    fn a_transfer_that_moves_no_byte_is_stopped_after_thirty_seconds_unless_tuned() {
        assert_eq!(Tuning::default().stall, Duration::from_secs(30));
    }
}
