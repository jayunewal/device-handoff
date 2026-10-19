// SPDX-License-Identifier: Apache-2.0

//! Helpers for the integration tests: devices that run inside the test process.
//!
//! Every device binds loopback only, has discovery and port mapping switched off,
//! and has short time limits, so tests are fast and need no network. Devices find
//! each other through the addresses in pairing tickets, as they would on a network
//! where discovery is blocked.
#![allow(
    dead_code,
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::field_reassign_with_default
)]

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use baton_core::{
    Config, ConfirmationCode, HandoffHandler, Identity, Node, PairApprovalRequest, PairedDevice,
    TransferHandler, TrustStore, Tuning, clock::ManualClock, config::RelayMode, util::BoxFuture,
};
use baton_proto::{
    DeviceId, DeviceInfo, Platform,
    message::{Handoff, Item, Outcome, capability},
};
use iroh::{
    Endpoint, EndpointAddr, SecretKey,
    endpoint::{Connection, presets},
    tls::CaTlsConfig,
};
use tokio::sync::Notify;

/// Where Unix time starts in these tests: some time in 2027.
pub const START: u64 = 1_800_000_000;

/// What a device was handed, in order.
#[derive(Debug, Default)]
pub struct Inbox {
    items: Mutex<Vec<(DeviceId, Item)>>,
    arrived: Notify,
}

impl Inbox {
    pub fn items(&self) -> Vec<(DeviceId, Item)> {
        self.items.lock().unwrap().clone()
    }

    /// Waits until `count` items have arrived.
    pub async fn wait_for(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let notified = self.arrived.notified();
                if self.items.lock().unwrap().len() >= count {
                    return;
                }
                notified.await;
            }
        })
        .await
        .expect("the items never arrived");
    }
}

/// Records every item and accepts it, if it is a kind the device advertises.
pub struct Recorder {
    pub inbox: Arc<Inbox>,
    pub accepts: &'static [&'static str],
}

impl HandoffHandler for Recorder {
    fn capabilities(&self) -> &'static [&'static str] {
        self.accepts
    }

    fn handle<'a>(&'a self, from: &'a PairedDevice, handoff: Handoff) -> BoxFuture<'a, Outcome> {
        Box::pin(async move {
            self.inbox
                .items
                .lock()
                .unwrap()
                .push((from.id, handoff.item));
            self.inbox.arrived.notify_waiters();
            Outcome::Accepted
        })
    }
}

/// Lets a test choose the handler a device starts with.
struct Chosen(Box<dyn HandoffHandler>);

impl HandoffHandler for Chosen {
    fn capabilities(&self) -> &'static [&'static str] {
        self.0.capabilities()
    }

    fn handle<'a>(&'a self, from: &'a PairedDevice, handoff: Handoff) -> BoxFuture<'a, Outcome> {
        self.0.handle(from, handoff)
    }
}

/// The ways a test device can differ from the default.
pub struct Spec {
    pub name: String,
    pub config: Config,
    pub tuning: Tuning,
    pub accepts: &'static [&'static str],
    pub clock: Arc<ManualClock>,
    /// Use only this relay, with no direct addresses, and trust its test certificate.
    pub relay_only: Option<String>,
    /// Waited for, for each session connection the gate is about to register.
    pub before_registering: Option<Arc<dyn Fn(DeviceId) -> BoxFuture<'static, ()> + Send + Sync>>,
    /// Takes the place of the recorder, which then stays empty.
    handler: Option<Box<dyn HandoffHandler>>,
    /// What takes the files that other devices send. A device without it takes none.
    transfers: Option<Arc<dyn TransferHandler>>,
}

impl Spec {
    pub fn new(name: &str) -> Self {
        let mut config = Config::default();
        config.device_name = Some(name.to_owned());
        config.network.relay = RelayMode::Disabled;
        config.network.lan_discovery = false;
        config.network.dns_discovery = false;
        config.network.port_mapping = false;

        let mut tuning = Tuning::default();
        tuning.invalid_delay = Duration::from_millis(10);
        tuning.request_deadline = Duration::from_secs(2);
        tuning.reply_deadline = Duration::from_secs(10);
        tuning.connect_timeout = Duration::from_secs(10);
        tuning.approval_timeout = Duration::from_secs(5);

        Self {
            name: name.to_owned(),
            config,
            tuning,
            accepts: &[capability::LINK, capability::CLIPBOARD_TEXT],
            clock: Arc::new(ManualClock::new(START)),
            relay_only: None,
            before_registering: None,
            handler: None,
            transfers: None,
        }
    }

    /// Hands what the device receives to `handler` instead of recording it.
    pub fn with_handler(mut self, handler: impl HandoffHandler) -> Self {
        self.handler = Some(Box::new(handler));
        self
    }

    /// Has the device take the files that other devices send, as `handler` decides.
    pub fn with_transfers(mut self, handler: Arc<dyn TransferHandler>) -> Self {
        self.transfers = Some(handler);
        self
    }

    /// Reaches the world only through the relay at `url`.
    pub fn through_relay(mut self, url: &str) -> Self {
        self.config.network.relay = RelayMode::Custom;
        self.config.network.relay_urls = vec![url.to_owned()];
        self.relay_only = Some(url.to_owned());
        self
    }

    pub async fn start(self) -> Device {
        let dir = tempfile::tempdir().unwrap();
        let identity = Identity::generate();
        self.start_with(dir, &identity).await
    }

    /// Starts a device with an identity that is already known, for tests that need its key.
    pub async fn start_with(self, dir: tempfile::TempDir, identity: &Identity) -> Device {
        let trust = TrustStore::load(dir.path().join("devices.json")).unwrap();
        let inbox = Arc::new(Inbox::default());
        let recorder = Recorder {
            inbox: Arc::clone(&inbox),
            accepts: self.accepts,
        };
        let relay_only = self.relay_only.is_some();
        let handler: Box<dyn HandoffHandler> = match self.handler {
            Some(handler) => handler,
            None => Box::new(recorder),
        };
        let mut builder = Node::builder(identity, trust.clone(), self.config)
            .handoff_handler(Chosen(handler))
            .clock(Arc::clone(&self.clock))
            .tuning(self.tuning);
        if let Some(hook) = self.before_registering {
            builder = builder.before_registering(move |peer| hook(peer));
        }
        if let Some(transfers) = self.transfers {
            builder = builder.transfer_handler(transfers);
        }
        let node = builder
            .customize_endpoint(move |builder| {
                if relay_only {
                    builder
                        .clear_ip_transports()
                        .ca_tls_config(CaTlsConfig::insecure_skip_verify())
                } else {
                    builder
                        .clear_ip_transports()
                        .bind_addr("127.0.0.1:0")
                        .expect("loopback is a valid address")
                }
            })
            .bind()
            .await
            .expect("the node starts");
        Device {
            id: identity.device_id(),
            name: self.name,
            node: Arc::new(node),
            trust,
            inbox,
            clock: self.clock,
            dir,
        }
    }
}

/// A running device and the handles a test needs to look inside it.
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    pub node: Arc<Node>,
    pub trust: TrustStore,
    pub inbox: Arc<Inbox>,
    pub clock: Arc<ManualClock>,
    dir: tempfile::TempDir,
}

impl Device {
    /// Stops the device and hands back its state directory, so the same device can be
    /// started again, for example under another name.
    pub async fn stop(self) -> tempfile::TempDir {
        self.node.shutdown().await;
        self.dir
    }

    /// Where a raw QUIC client can dial this device.
    pub fn addr(&self) -> EndpointAddr {
        let key = iroh::PublicKey::from_bytes(self.id.as_bytes()).unwrap();
        self.node
            .status()
            .direct_addrs
            .into_iter()
            .fold(EndpointAddr::new(key), EndpointAddr::with_ip_addr)
    }

    /// Pairs this device with `other` the way people do, with `other` approving.
    ///
    /// Returns what each screen showed. The person compares the two, so a test that
    /// pairs has to as well: this panics if they differ, or if the pairing fails.
    pub async fn pair_with(&self, other: &Device) -> Codes {
        self.try_pair_with(other).await.unwrap()
    }

    /// Like [`Device::pair_with`], but says so when this device cannot reach `other`, for
    /// a test that expects to be turned away for a while and to try again.
    pub async fn try_pair_with(&self, other: &Device) -> Result<Codes, baton_core::Error> {
        let on_offerer = Arc::new(Mutex::new(None));
        let offer = {
            let on_offerer = Arc::clone(&on_offerer);
            other
                .node
                .offer_pairing(move |request: PairApprovalRequest| {
                    *on_offerer.lock().unwrap() = Some(request.confirmation);
                    std::future::ready(true)
                })
                .await
                .unwrap()
        };
        let ticket = offer.ticket().clone();
        let mut on_joiner = None;
        self.node
            .join_pairing(&ticket, |code| on_joiner = Some(code))
            .await?;
        offer.wait().await.unwrap();
        assert!(self.trust.contains(&other.id) && other.trust.contains(&self.id));
        let codes = Codes {
            offerer: on_offerer
                .lock()
                .unwrap()
                .expect("the offerer was shown a code"),
            joiner: on_joiner.expect("the joiner was shown a code"),
        };
        assert_eq!(
            codes.offerer, codes.joiner,
            "the two screens show different codes"
        );
        Ok(codes)
    }
}

/// The confirmation code each device showed during a pairing.
#[derive(Debug, Clone, Copy)]
pub struct Codes {
    pub offerer: ConfirmationCode,
    pub joiner: ConfirmationCode,
}

pub async fn device(name: &str) -> Device {
    Spec::new(name).start().await
}

/// Two devices that are paired with each other.
pub async fn pair() -> (Device, Device) {
    let a = device("Laptop").await;
    let b = device("Phone").await;
    b.pair_with(&a).await;
    (a, b)
}

/// An approver that says yes.
pub fn approve(_request: PairApprovalRequest) -> std::future::Ready<bool> {
    std::future::ready(true)
}

/// An approver that says no.
pub fn refuse(_request: PairApprovalRequest) -> std::future::Ready<bool> {
    std::future::ready(false)
}

pub fn info(name: &str) -> DeviceInfo {
    DeviceInfo {
        name: name.to_owned(),
        platform: Platform::Linux,
        app_version: "0.0.0-test".to_owned(),
    }
}

/// A QUIC client that speaks to a device at the lowest level, for tests that need
/// to send things a well-behaved device never would.
pub struct Raw {
    pub endpoint: Endpoint,
    pub id: DeviceId,
}

impl Raw {
    pub async fn new() -> Self {
        Self::with_key(SecretKey::generate()).await
    }

    pub async fn with_key(secret: SecretKey) -> Self {
        let id = DeviceId::from_bytes(*secret.public().as_bytes());
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(secret)
            .clear_ip_transports()
            .bind_addr("127.0.0.1:0")
            .unwrap()
            .bind()
            .await
            .unwrap();
        Self { endpoint, id }
    }

    pub async fn dial(&self, target: &Device, alpn: &[u8]) -> Result<Connection, String> {
        tokio::time::timeout(
            Duration::from_secs(10),
            self.endpoint.connect(target.addr(), alpn),
        )
        .await
        .map_err(|_| "timed out".to_owned())?
        .map_err(|e| e.to_string())
    }
}

/// How long [`retrying`] keeps at it: past the 10 seconds in which a device's allowance for
/// one sender's connection attempts refills.
const RETRY_WITHIN: Duration = Duration::from_secs(15);

/// Runs `attempt` until it works, and panics if it still does not after [`RETRY_WITHIN`].
///
/// For what a test does right after flooding a device with connection attempts. Every device in
/// these tests has the same address, so until the allowance that address was held to has
/// refilled, a device turns the next sender away as it did the flood. How long that takes
/// depends on how busy the machine is, so a fixed wait would either flake or be slow.
pub async fn retrying<T, E, Fut>(what: &str, mut attempt: impl FnMut() -> Fut) -> T
where
    E: std::fmt::Display,
    Fut: Future<Output = Result<T, E>>,
{
    let started = tokio::time::Instant::now();
    loop {
        match attempt().await {
            Ok(done) => return done,
            Err(_) if started.elapsed() < RETRY_WITHIN => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => panic!("{what} still fails after {RETRY_WITHIN:?}: {error}"),
        }
    }
}

/// Waits for `condition` to become true, or panics.
pub async fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..200 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("gave up waiting for {what}");
}

/// A frame header for a payload length, as a peer would send it.
pub fn header(len: u32) -> [u8; 4] {
    len.to_be_bytes()
}
