// SPDX-License-Identifier: Apache-2.0

//! Real nodes on loopback, for the tests of this crate that reach into its private parts, such as
//! the pool of connections or the task that takes unidirectional streams.
//!
//! The tests in `tests/` have their own helpers, built on the public interface.

use baton_proto::{DeviceId, DeviceInfo, Platform};

use crate::{Config, ContactHints, Identity, Node, NodeBuilder, TrustStore, config::RelayMode};

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
