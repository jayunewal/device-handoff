// SPDX-License-Identifier: Apache-2.0

//! Real nodes and bare devices on loopback, for the tests of this crate that reach into its private
//! parts, such as the pool of connections or the task that takes unidirectional streams.
//!
//! The tests in `tests/` have their own helpers, built on the public interface.

use std::{sync::Arc, time::Duration};

use baton_proto::{ALPN_SESSION, DeviceId, DeviceInfo, Platform};
use iroh::{
    Endpoint, EndpointAddr, SecretKey,
    endpoint::{Connection, SendStream, presets},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    Config, ContactHints, Identity, Node, NodeBuilder, TrustStore,
    config::RelayMode,
    gate::{ConnectionRegistry, Gate},
    transfer::incoming::Link,
    transport,
    uni::{Arrival, Kind, Sink, Sinks, Uni},
};

/// A node that listens on loopback only, and what a test needs to reach into it.
pub(crate) struct TestNode {
    pub(crate) node: Node,
    pub(crate) id: DeviceId,
    pub(crate) trust: TrustStore,
    _dir: tempfile::TempDir,
}

impl TestNode {
    /// A node with the settings that every test uses.
    pub(crate) async fn start() -> Self {
        Self::with(|builder| builder).await
    }

    /// A node whose builder `build` has changed first, for example to give it a `Tuning`.
    pub(crate) async fn with(build: impl FnOnce(NodeBuilder) -> NodeBuilder) -> Self {
        Self::configured(|_| {}, build).await
    }

    /// A node whose settings `configure` has changed, and then its builder `build`.
    pub(crate) async fn configured(
        configure: impl FnOnce(&mut Config),
        build: impl FnOnce(NodeBuilder) -> NodeBuilder,
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let identity = Identity::generate();
        let trust = TrustStore::load(dir.path().join("devices.json")).unwrap();
        let mut config = Config::default();
        config.network.relay = RelayMode::Disabled;
        config.network.lan_discovery = false;
        config.network.dns_discovery = false;
        config.network.port_mapping = false;
        configure(&mut config);
        let builder = build(NodeBuilder::new(&identity, trust.clone(), config));
        let node = builder
            .customize_endpoint(|builder| {
                builder
                    .clear_ip_transports()
                    .bind_addr("127.0.0.1:0")
                    .expect("loopback is a valid address")
            })
            .bind()
            .await
            .unwrap();
        Self {
            node,
            id: identity.device_id(),
            trust,
            _dir: dir,
        }
    }

    /// Makes the two nodes paired devices that know where to find each other.
    pub(crate) fn pair_with(&self, other: &Self) {
        for (trust, peer) in [(&self.trust, other), (&other.trust, self)] {
            let info = DeviceInfo {
                name: "Device".to_owned(),
                platform: Platform::Linux,
                app_version: "0.0.0-test".to_owned(),
            };
            trust.pair(peer.id, &info, 1).unwrap();
            let hints = ContactHints {
                relay: None,
                addrs: peer.node.status().direct_addrs,
            };
            trust.record_contact(&peer.id, 1, &hints).unwrap();
        }
    }
}

pub(crate) const PATIENCE: Duration = Duration::from_secs(10);

/// Long enough for something that should not happen to have happened.
pub(crate) const QUIET: Duration = Duration::from_millis(400);

/// Passes on what it is given.
pub(crate) struct Collect(mpsc::UnboundedSender<Arrival>);

impl Sink for Collect {
    fn take(&self, arrival: Arrival) {
        let _ = self.0.send(arrival);
    }
}

pub(crate) fn collector() -> (Arc<dyn Sink>, mpsc::UnboundedReceiver<Arrival>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    (Arc::new(Collect(sender)), receiver)
}

pub(crate) async fn next(receiver: &mut mpsc::UnboundedReceiver<Arrival>) -> Arrival {
    tokio::time::timeout(PATIENCE, receiver.recv())
        .await
        .expect("no stream arrived")
        .expect("the sink was dropped")
}

pub(crate) async fn nothing_arrives(receiver: &mut mpsc::UnboundedReceiver<Arrival>) {
    assert!(
        tokio::time::timeout(QUIET, receiver.recv()).await.is_err(),
        "a stream arrived that should have been stopped"
    );
}

pub(crate) fn info() -> DeviceInfo {
    DeviceInfo {
        name: "Device".to_owned(),
        platform: Platform::Linux,
        app_version: "0.0.0-test".to_owned(),
    }
}

/// A bare endpoint on loopback behind the gate, as a node has it, with the dispatcher for
/// the connections that it makes and accepts.
pub(crate) struct Device {
    pub(crate) endpoint: Endpoint,
    pub(crate) trust: TrustStore,
    pub(crate) uni: Arc<Uni>,
    pub(crate) id: DeviceId,
    _dir: tempfile::TempDir,
}

impl Device {
    pub(crate) async fn new(sinks: Sinks, start_deadline: Duration) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let secret = SecretKey::generate();
        let trust = TrustStore::load(dir.path().join("devices.json")).unwrap();
        let uni = Arc::new(Uni::new(
            sinks,
            &trust,
            &CancellationToken::new(),
            start_deadline,
        ));
        let gate = Gate::new(
            trust.clone(),
            Arc::new(ConnectionRegistry::default()),
            uni.quic_limit(),
        );
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(secret.clone())
            .alpns(vec![ALPN_SESSION.to_vec()])
            .hooks(gate)
            .transport_config(transport::config())
            .clear_ip_transports()
            .bind_addr("127.0.0.1:0")
            .unwrap()
            .bind()
            .await
            .unwrap();
        Self {
            endpoint,
            trust,
            uni,
            id: DeviceId::from_bytes(*secret.public().as_bytes()),
            _dir: dir,
        }
    }

    /// A device that takes the streams of the kinds in `kinds`, with their limits.
    pub(crate) async fn taking(
        kinds: &[(Kind, u32)],
    ) -> (Self, Vec<mpsc::UnboundedReceiver<Arrival>>) {
        Self::taking_within(kinds, PATIENCE).await
    }

    pub(crate) async fn taking_within(
        kinds: &[(Kind, u32)],
        start_deadline: Duration,
    ) -> (Self, Vec<mpsc::UnboundedReceiver<Arrival>>) {
        let mut sinks = Sinks::default();
        let mut receivers = Vec::new();
        for &(kind, limit) in kinds {
            let (sink, receiver) = collector();
            sinks = sinks.install(kind, limit, sink);
            receivers.push(receiver);
        }
        (Self::new(sinks, start_deadline).await, receivers)
    }

    pub(crate) fn pair_with(&self, other: &Self) {
        self.trust.pair(other.id, &info(), 1).unwrap();
    }
}

/// Pairs two devices, connects `from` to `to`, and has each end take the streams of its
/// connection, as the session code does. Returns the connection as each end has it.
pub(crate) async fn connect(from: &Device, to: &Device) -> (Connection, Connection) {
    from.pair_with(to);
    to.pair_with(from);
    let accepting = tokio::spawn({
        let endpoint = to.endpoint.clone();
        async move { endpoint.accept().await.unwrap().await.unwrap() }
    });
    let address = EndpointAddr::new(to.endpoint.id()).with_ip_addr(to.endpoint.bound_sockets()[0]);
    let dialed = from.endpoint.connect(address, ALPN_SESSION).await.unwrap();
    let accepted = accepting.await.unwrap();
    from.uni.spawn(dialed.clone(), Link(1));
    to.uni.spawn(accepted.clone(), Link(2));
    (dialed, accepted)
}

/// Opens a stream and writes `bytes` on it. The stream stays open.
pub(crate) async fn open(connection: &Connection, bytes: &[u8]) -> SendStream {
    let mut send = tokio::time::timeout(PATIENCE, connection.open_uni())
        .await
        .expect("the stream could not be opened")
        .unwrap();
    send.write_all(bytes).await.unwrap();
    send
}

/// The code that the other end stopped `send` with.
pub(crate) async fn stopped_with(send: &SendStream) -> u64 {
    tokio::time::timeout(PATIENCE, send.stopped())
        .await
        .expect("the stream was never stopped")
        .unwrap()
        .expect("the stream was finished and not stopped")
        .into_inner()
}
