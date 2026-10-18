// SPDX-License-Identifier: Apache-2.0

//! Deciding who may connect and who may stay connected.
//!
//! The transport authenticates every peer by its key, but a key being genuine
//! says nothing about whether we want to talk to it. Two checks sit in front of
//! the protocol handlers:
//!
//! - [`Admission`] runs on the first packet of an incoming connection, before the
//!   handshake (so before any public-key cryptography or state for the connection),
//!   and sheds floods cheaply.
//! - [`Gate`] runs right after the handshake, before any data is read, and closes
//!   session connections from devices that are not paired.
//!
//! [`ConnectionRegistry`] remembers the open session connections, in both
//! directions, so that unpairing a device can close them at once.

use std::{
    collections::HashMap,
    fmt,
    net::{IpAddr, Ipv6Addr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use baton_proto::{ALPN_PAIR, ALPN_SESSION, DeviceId, close_code};
use iroh::{
    EndpointId,
    endpoint::{
        AfterHandshakeOutcome, Connection, EndpointHooks, Incoming, IncomingAddr, VarInt,
        WeakConnectionHandle,
    },
    protocol::IncomingFilterOutcome,
};

#[cfg(any(test, feature = "test-support"))]
use crate::util::BoxFuture;
use crate::{ratelimit::RateLimiter, transport, trust::TrustStore, util::lock};

/// Most session connections kept open to or from one paired device. A well-behaved
/// device needs one; the margin is for reconnects that overlap.
pub(crate) const MAX_CONNECTIONS_PER_PEER: usize = 8;

/// Handshakes from sources that have not proven their address that are let
/// through each second without asking them to. Beyond this the node asks for proof
/// first, which costs it nothing but a small reply.
const UNPROVEN_PER_SECOND: u32 = 64;

/// Connection attempts allowed from one proven source per [`SOURCE_WINDOW`]. Relayed
/// keys that nobody has paired with get this much between them, not each.
const SOURCE_BURST: u32 = 20;
const SOURCE_WINDOW: Duration = Duration::from_secs(10);
/// Sources remembered at once. Old ones are forgotten first.
const SOURCE_KEYS: usize = 1024;

/// What an IPv6 sender is held to account by: its /64. Almost every IPv6 host can use any
/// address in the /64 its network gives it, so each address would otherwise be a new sender.
const IPV6_PREFIX_MASK: u128 = !0 << 64;

/// The device on the other end of a connection, as a Baton id.
pub(crate) fn remote_device(connection: &Connection) -> DeviceId {
    DeviceId::from_bytes(*connection.remote_id().as_bytes())
}

pub(crate) fn code(code: u32) -> VarInt {
    VarInt::from_u32(code)
}

/// The open session connections, by the device on the other end.
#[derive(Debug, Default)]
pub(crate) struct ConnectionRegistry {
    by_peer: Mutex<HashMap<DeviceId, Vec<WeakConnectionHandle>>>,
}

impl ConnectionRegistry {
    /// Notes a new connection. Handles to connections that have ended are dropped
    /// on the way, and if the device has too many open the oldest is closed.
    pub(crate) fn register(&self, peer: DeviceId, handle: WeakConnectionHandle) {
        let surplus = {
            let mut map = lock(&self.by_peer);
            let handles = map.entry(peer).or_default();
            handles.retain(is_open);
            let surplus: Vec<_> = if handles.len() >= MAX_CONNECTIONS_PER_PEER {
                let excess = handles.len() + 1 - MAX_CONNECTIONS_PER_PEER;
                handles.drain(..excess).collect()
            } else {
                Vec::new()
            };
            handles.push(handle);
            surplus
        };
        close_all(&surplus, close_code::OVERLOADED, b"too many connections");
    }

    /// Closes every connection to `peer`. Returns how many were still open.
    pub(crate) fn close_peer(&self, peer: &DeviceId, error_code: u32, reason: &[u8]) -> usize {
        let handles = lock(&self.by_peer).remove(peer).unwrap_or_default();
        close_all(&handles, error_code, reason)
    }

    /// Closes the connections of every peer for which `keep` says no.
    ///
    /// Used when the list of paired devices changes in a way the events did not
    /// describe, for example after a receiver fell behind.
    pub(crate) fn close_unless(
        &self,
        keep: impl Fn(&DeviceId) -> bool,
        error_code: u32,
        reason: &[u8],
    ) -> usize {
        let doomed: Vec<_> = {
            let mut map = lock(&self.by_peer);
            let peers: Vec<DeviceId> = map.keys().filter(|id| !keep(id)).copied().collect();
            peers
                .into_iter()
                .filter_map(|id| map.remove(&id))
                .flatten()
                .collect()
        };
        close_all(&doomed, error_code, reason)
    }

    /// Closes everything, for shutdown.
    pub(crate) fn close_everything(&self, error_code: u32, reason: &[u8]) {
        let all: Vec<_> = lock(&self.by_peer)
            .drain()
            .flat_map(|(_, handles)| handles)
            .collect();
        close_all(&all, error_code, reason);
    }

    /// The peers with at least one open connection, and how many each has.
    pub(crate) fn open_counts(&self) -> Vec<(DeviceId, usize)> {
        let mut map = lock(&self.by_peer);
        map.retain(|_, handles| {
            handles.retain(is_open);
            !handles.is_empty()
        });
        map.iter()
            .map(|(id, handles)| (*id, handles.len()))
            .collect()
    }
}

fn is_open(handle: &WeakConnectionHandle) -> bool {
    handle
        .upgrade()
        .is_some_and(|connection| connection.close_reason().is_none())
}

fn close_all(handles: &[WeakConnectionHandle], error_code: u32, reason: &[u8]) -> usize {
    handles
        .iter()
        .filter_map(WeakConnectionHandle::upgrade)
        .filter(|connection| connection.close_reason().is_none())
        .map(|connection| connection.close(code(error_code), reason))
        .count()
}

/// See [`Gate::before_registering`].
#[cfg(any(test, feature = "test-support"))]
pub(crate) type BeforeRegistering = Arc<dyn Fn(DeviceId) -> BoxFuture<'static, ()> + Send + Sync>;

/// Closes session connections from devices that are not paired.
///
/// The hook is installed on the endpoint, so it must never hold the endpoint
/// itself: that would be a reference cycle.
#[derive(Clone)]
pub(crate) struct Gate {
    trust: TrustStore,
    registry: Arc<ConnectionRegistry>,
    /// How many unidirectional streams a paired device may have open at once, which is nothing
    /// until a kind of stream is taken (see [`crate::uni`]).
    uni_streams: u32,
    /// Runs after a connection has passed the first check and before it is registered,
    /// the one moment at which a device can be unpaired without the registry knowing the
    /// connection exists. Tests use it to unpair a device at exactly that moment, and to
    /// let the task that reacts to the unpairing run before the connection is registered.
    #[cfg(any(test, feature = "test-support"))]
    before_registering: Option<BeforeRegistering>,
}

impl fmt::Debug for Gate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Gate").finish_non_exhaustive()
    }
}

impl Gate {
    pub(crate) fn new(
        trust: TrustStore,
        registry: Arc<ConnectionRegistry>,
        uni_streams: u32,
    ) -> Self {
        Self {
            trust,
            registry,
            uni_streams,
            #[cfg(any(test, feature = "test-support"))]
            before_registering: None,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn before_registering(mut self, hook: BeforeRegistering) -> Self {
        self.before_registering = Some(hook);
        self
    }

    fn refuse_unpaired() -> AfterHandshakeOutcome {
        AfterHandshakeOutcome::Reject {
            error_code: code(close_code::NOT_PAIRED),
            reason: b"not paired".to_vec(),
        }
    }
}

impl EndpointHooks for Gate {
    // Only the test hook awaits anything, so a build without it has an `async fn` with no
    // `.await`. The trait fixes the signature.
    #[cfg_attr(
        not(any(test, feature = "test-support")),
        allow(clippy::unused_async_trait_impl)
    )]
    async fn after_handshake<'a>(&'a self, connection: &'a Connection) -> AfterHandshakeOutcome {
        let alpn = connection.alpn();
        if alpn == ALPN_PAIR {
            // Pairing is open to anyone by design; what a pairing request is
            // allowed to do is decided by the pairing handler.
            return AfterHandshakeOutcome::Accept;
        }
        if alpn != ALPN_SESSION {
            return AfterHandshakeOutcome::Reject {
                error_code: code(close_code::PROTOCOL_VIOLATION),
                reason: b"unknown protocol".to_vec(),
            };
        }

        // Strangers are turned away without leaving anything behind: anyone can
        // make up keys, so the registry must only ever hold paired devices.
        let peer = remote_device(connection);
        if !self.trust.contains(&peer) {
            return Self::refuse_unpaired();
        }
        #[cfg(any(test, feature = "test-support"))]
        if let Some(hook) = &self.before_registering {
            hook(peer).await;
        }
        // Register first and check again second. If the device is unpaired in
        // between, either the unpairing event finds this connection and closes
        // it, or the second check sees the device gone; checked only before
        // registering, a connection could slip through unnoticed.
        self.registry.register(peer, connection.weak_handle());
        if self.trust.contains(&peer) {
            // Until now the connection could make this device hold only a small window of
            // data, and open no stream but a request. The other end is paired, so it gets the
            // window meant for paired devices and the streams that this device takes.
            transport::raise_window(connection);
            if self.uni_streams > 0 {
                transport::raise_uni_streams(connection, self.uni_streams);
            }
            AfterHandshakeOutcome::Accept
        } else {
            Self::refuse_unpaired()
        }
    }
}

/// Who a connection attempt is held to account as, once that can be told.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Source {
    /// A UDP sender: its address, or for IPv6 the /64 it belongs to.
    Network(IpAddr),
    /// The key of a paired device, seen through a relay. Only paired keys are tracked one by
    /// one: anyone can make a new key for each attempt, so a limit on each of those would not
    /// hold anyone back, and the table would fill up with keys that are never seen again.
    Paired(EndpointId),
}

/// Decides whether an incoming connection attempt is worth a handshake.
///
/// This runs on the first packet, before the handshake: before any public-key
/// cryptography is done or any state for the connection is kept. (noq has by then derived
/// the Initial keys, which come from a public salt and the connection id, removed header
/// protection and checked any retry token.) It has to be cheap and it has to be careful
/// about what it believes:
///
/// - The source address of a UDP packet can be forged, so an address is only held to
///   account once it has answered a retry token. Limiting one that has not would let anyone
///   on the internet lock a real device out by forging its address.
/// - An address that has answered one is held to account as a whole for IPv4, and as its
///   /64 for IPv6, where a sender has far more addresses than any limit could cover.
/// - A relay checks that the sender holds its key, but keys are free. A paired device's key
///   has an allowance of its own, which no stranger can use up; every other key shares one,
///   and is asked to answer a retry token first when attempts come faster than the node
///   wants to handle, like an address that has not proven itself.
#[derive(Debug)]
pub(crate) struct Admission {
    trust: TrustStore,
    /// Attempts from sources that have not answered a retry token: UDP addresses, and relayed
    /// keys that nobody has paired with.
    unproven: Mutex<RateLimiter<()>>,
    /// Attempts from networks that have answered one, and from paired devices' keys.
    sources: Mutex<RateLimiter<Source>>,
    /// Attempts from relayed keys nobody has paired with, which have answered one, all together.
    strangers: Mutex<RateLimiter<()>>,
}

impl Admission {
    pub(crate) fn new(trust: TrustStore) -> Self {
        Self {
            trust,
            unproven: Mutex::new(RateLimiter::new(
                UNPROVEN_PER_SECOND,
                Duration::from_secs(1),
                1,
            )),
            sources: Mutex::new(RateLimiter::new(SOURCE_BURST, SOURCE_WINDOW, SOURCE_KEYS)),
            strangers: Mutex::new(RateLimiter::new(SOURCE_BURST, SOURCE_WINDOW, 1)),
        }
    }

    pub(crate) fn judge(&self, incoming: &Incoming) -> IncomingFilterOutcome {
        self.judge_at(
            &incoming.remote_addr(),
            incoming.remote_addr_validated(),
            Instant::now(),
        )
    }

    fn judge_at(
        &self,
        addr: &IncomingAddr,
        validated: bool,
        now: Instant,
    ) -> IncomingFilterOutcome {
        let source = match addr {
            IncomingAddr::Ip(addr) => Source::Network(network_of(addr.ip())),
            IncomingAddr::Relay { endpoint_id, .. } => {
                // The relay has already checked that the sender holds this key, but not
                // that it is one we know. Whether it is paired is looked up in the same
                // list the gate uses once the handshake is done.
                if !self
                    .trust
                    .contains(&DeviceId::from_bytes(*endpoint_id.as_bytes()))
                {
                    return self.judge_stranger(validated, now);
                }
                Source::Paired(*endpoint_id)
            }
            // No other transport is enabled.
            _ => return IncomingFilterOutcome::Reject,
        };
        if matches!(source, Source::Network(_)) && !validated {
            return self.judge_unproven(now);
        }
        if lock(&self.sources).allow(&source, now) {
            IncomingFilterOutcome::Accept
        } else {
            IncomingFilterOutcome::Reject
        }
    }

    /// Quiet times cost nothing. When many unproven attempts arrive at once, asking for a
    /// retry token makes each of them prove its address first, and a forged address never
    /// sees the question.
    fn judge_unproven(&self, now: Instant) -> IncomingFilterOutcome {
        if lock(&self.unproven).allow(&(), now) {
            IncomingFilterOutcome::Accept
        } else {
            IncomingFilterOutcome::Retry
        }
    }

    /// A relayed key that is not paired. It is treated like an address that has not proven
    /// itself, and once it has answered a retry token it draws on an allowance that every
    /// such key shares, because a fresh key costs nothing. Pairing over a relay goes
    /// through this allowance, which is enough for something that happens this rarely.
    fn judge_stranger(&self, validated: bool, now: Instant) -> IncomingFilterOutcome {
        if !validated {
            return self.judge_unproven(now);
        }
        if lock(&self.strangers).allow(&(), now) {
            IncomingFilterOutcome::Accept
        } else {
            IncomingFilterOutcome::Reject
        }
    }
}

/// What a sender at `addr` is held to account by: the address, or its /64 for IPv6.
/// An IPv4 address written as IPv6 is the same sender as the IPv4 address.
fn network_of(addr: IpAddr) -> IpAddr {
    match addr.to_canonical() {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from(u128::from(v6) & IPV6_PREFIX_MASK)),
    }
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, ops::Deref};

    use baton_proto::{DeviceInfo, Platform};
    use iroh::{Endpoint, EndpointAddr, RelayUrl, SecretKey, endpoint::presets};

    use super::*;

    /// An `Admission` with a list of paired devices of its own.
    struct Fixture {
        admission: Admission,
        trust: TrustStore,
        _dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let trust = TrustStore::load(dir.path().join("devices.json")).unwrap();
            Self {
                admission: Admission::new(trust.clone()),
                trust,
                _dir: dir,
            }
        }

        /// Pairs with the device that reaches this one through `relayed(seed)`.
        fn pair_with(&self, seed: u8) {
            let key = SecretKey::from_bytes(&[seed; 32]).public();
            let info = DeviceInfo {
                name: format!("Device {seed}"),
                platform: Platform::Linux,
                app_version: "0.0.0-test".to_owned(),
            };
            self.trust
                .pair(DeviceId::from_bytes(*key.as_bytes()), &info, 1_800_000_000)
                .unwrap();
        }
    }

    impl Deref for Fixture {
        type Target = Admission;

        fn deref(&self) -> &Admission {
            &self.admission
        }
    }

    fn ip(last: u8) -> IncomingAddr {
        IncomingAddr::Ip(SocketAddr::from(([192, 168, 1, last], 4433)))
    }

    fn ip6(text: &str) -> IncomingAddr {
        IncomingAddr::Ip(SocketAddr::new(text.parse().unwrap(), 4433))
    }

    fn relayed(seed: u8) -> IncomingAddr {
        IncomingAddr::Relay {
            url: "https://relay.example".parse::<RelayUrl>().unwrap(),
            endpoint_id: SecretKey::from_bytes(&[seed; 32]).public(),
        }
    }

    #[test]
    fn quiet_unproven_sources_are_let_through() {
        let admission = Fixture::new();
        let now = Instant::now();
        for _ in 0..UNPROVEN_PER_SECOND {
            assert_eq!(
                admission.judge_at(&ip(1), false, now),
                IncomingFilterOutcome::Accept
            );
        }
    }

    #[test]
    fn a_flood_of_unproven_sources_is_asked_to_prove_itself() {
        let admission = Fixture::new();
        let now = Instant::now();
        let mut accepted = 0;
        let mut retried = 0;
        for i in 0..1_000u32 {
            let last = u8::try_from(i % 250).unwrap();
            match admission.judge_at(&ip(last), false, now) {
                IncomingFilterOutcome::Accept => accepted += 1,
                IncomingFilterOutcome::Retry => retried += 1,
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(accepted, UNPROVEN_PER_SECOND);
        assert_eq!(retried, 1_000 - UNPROVEN_PER_SECOND);
    }

    #[test]
    fn forged_attempts_never_use_up_a_real_sources_allowance() {
        let admission = Fixture::new();
        let now = Instant::now();
        // Many unproven attempts claiming to come from 192.168.1.9.
        for _ in 0..500 {
            admission.judge_at(&ip(9), false, now);
        }
        // The real device, once it has answered a retry token, is not held back.
        for _ in 0..SOURCE_BURST {
            assert_eq!(
                admission.judge_at(&ip(9), true, now),
                IncomingFilterOutcome::Accept
            );
        }
    }

    #[test]
    fn a_proven_source_that_floods_is_refused_and_others_are_not() {
        let admission = Fixture::new();
        let now = Instant::now();
        for _ in 0..SOURCE_BURST {
            assert_eq!(
                admission.judge_at(&ip(1), true, now),
                IncomingFilterOutcome::Accept
            );
        }
        assert_eq!(
            admission.judge_at(&ip(1), true, now),
            IncomingFilterOutcome::Reject
        );
        assert_eq!(
            admission.judge_at(&ip(2), true, now),
            IncomingFilterOutcome::Accept
        );
        // And it is welcome again once the window has passed.
        assert_eq!(
            admission.judge_at(&ip(1), true, now + SOURCE_WINDOW),
            IncomingFilterOutcome::Accept
        );
    }

    #[test]
    fn an_ipv6_sender_is_limited_by_the_prefix_it_can_rotate_through() {
        let admission = Fixture::new();
        let now = Instant::now();
        let mut accepted = 0;
        for host in 0..200u32 {
            let addr = ip6(&format!("2001:db8:1:2::{host:x}"));
            if admission.judge_at(&addr, true, now) == IncomingFilterOutcome::Accept {
                accepted += 1;
            }
        }
        assert_eq!(
            accepted, SOURCE_BURST,
            "every address in a /64 got its own allowance"
        );
        // Another /64 is another sender.
        assert_eq!(
            admission.judge_at(&ip6("2001:db8:1:3::1"), true, now),
            IncomingFilterOutcome::Accept
        );
    }

    #[test]
    fn an_ipv4_address_in_ipv6_form_is_the_same_sender() {
        let admission = Fixture::new();
        let now = Instant::now();
        for _ in 0..SOURCE_BURST {
            assert_eq!(
                admission.judge_at(&ip(7), true, now),
                IncomingFilterOutcome::Accept
            );
        }
        assert_eq!(
            admission.judge_at(&ip6("::ffff:192.168.1.7"), true, now),
            IncomingFilterOutcome::Reject
        );
    }

    #[test]
    fn relayed_keys_nobody_has_paired_with_share_one_allowance() {
        let admission = Fixture::new();
        let now = Instant::now();
        // Keys cost nothing to make, so a sender that rotates through them must not
        // get a fresh allowance for each.
        let mut accepted = 0;
        for seed in 0..200u8 {
            if admission.judge_at(&relayed(seed), true, now) == IncomingFilterOutcome::Accept {
                accepted += 1;
            }
        }
        assert_eq!(accepted, SOURCE_BURST);
        // The allowance is the same for a key that has not been seen before.
        assert_eq!(
            admission.judge_at(&relayed(250), true, now),
            IncomingFilterOutcome::Reject
        );
    }

    #[test]
    fn relayed_keys_nobody_has_paired_with_are_asked_to_prove_themselves_in_a_flood() {
        let admission = Fixture::new();
        let now = Instant::now();
        let mut accepted = 0;
        let mut retried = 0;
        for i in 0..1_000u32 {
            let seed = u8::try_from(i % 250).unwrap();
            match admission.judge_at(&relayed(seed), false, now) {
                IncomingFilterOutcome::Accept => accepted += 1,
                IncomingFilterOutcome::Retry => retried += 1,
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(accepted, UNPROVEN_PER_SECOND);
        assert_eq!(retried, 1_000 - UNPROVEN_PER_SECOND);
    }

    #[test]
    fn quiet_times_cost_relayed_strangers_nothing() {
        let admission = Fixture::new();
        let now = Instant::now();
        // Strangers that arrive at an ordinary pace are let through without being asked
        // for anything, and do not use up what the ones that were asked to retry share.
        for seed in 0..40u8 {
            assert_eq!(
                admission.judge_at(&relayed(seed), false, now),
                IncomingFilterOutcome::Accept
            );
        }
        for _ in 0..SOURCE_BURST {
            assert_eq!(
                admission.judge_at(&relayed(200), true, now),
                IncomingFilterOutcome::Accept
            );
        }
    }

    #[test]
    fn a_paired_devices_relayed_key_has_an_allowance_of_its_own() {
        let admission = Fixture::new();
        admission.pair_with(1);
        admission.pair_with(2);
        let now = Instant::now();

        // Strangers use up everything they share, and everything that is let through
        // without a question.
        for i in 0..1_000u32 {
            let seed = u8::try_from(100 + i % 100).unwrap();
            admission.judge_at(&relayed(seed), i % 2 == 0, now);
        }
        assert_eq!(
            admission.judge_at(&relayed(150), true, now),
            IncomingFilterOutcome::Reject
        );
        assert_eq!(
            admission.judge_at(&relayed(151), false, now),
            IncomingFilterOutcome::Retry
        );

        // A paired device is not held back by any of that, and is never asked to answer a
        // retry token, whether or not it has.
        for _ in 0..SOURCE_BURST {
            assert_eq!(
                admission.judge_at(&relayed(1), false, now),
                IncomingFilterOutcome::Accept
            );
        }
        // One that floods is refused, and another paired device is not affected.
        assert_eq!(
            admission.judge_at(&relayed(1), false, now),
            IncomingFilterOutcome::Reject
        );
        assert_eq!(
            admission.judge_at(&relayed(2), false, now),
            IncomingFilterOutcome::Accept
        );
    }

    #[test]
    fn a_device_that_is_unpaired_is_a_stranger_from_then_on() {
        let admission = Fixture::new();
        admission.pair_with(1);
        let now = Instant::now();
        for _ in 0..SOURCE_BURST {
            assert_eq!(
                admission.judge_at(&relayed(1), true, now),
                IncomingFilterOutcome::Accept
            );
        }
        assert_eq!(
            admission.judge_at(&relayed(1), true, now),
            IncomingFilterOutcome::Reject
        );

        let key = SecretKey::from_bytes(&[1; 32]).public();
        admission
            .trust
            .remove(&DeviceId::from_bytes(*key.as_bytes()))
            .unwrap();
        // It no longer has an allowance of its own, and shares the strangers' instead,
        // which is untouched.
        assert_eq!(
            admission.judge_at(&relayed(1), true, now),
            IncomingFilterOutcome::Accept
        );
        assert_eq!(
            admission.judge_at(&relayed(1), false, now),
            IncomingFilterOutcome::Accept
        );
    }

    /// A bare endpoint on loopback that uses the gate, as a node does.
    async fn gated_endpoint(secret: SecretKey, trust: TrustStore) -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .secret_key(secret)
            .alpns(vec![ALPN_SESSION.to_vec(), ALPN_PAIR.to_vec()])
            .hooks(Gate::new(trust, Arc::new(ConnectionRegistry::default()), 0))
            .transport_config(transport::config())
            .clear_ip_transports()
            .bind_addr("127.0.0.1:0")
            .unwrap()
            .bind()
            .await
            .unwrap()
    }

    fn device_info() -> DeviceInfo {
        DeviceInfo {
            name: "Device".to_owned(),
            platform: Platform::Linux,
            app_version: "0.0.0-test".to_owned(),
        }
    }

    /// Waits until `connection` has been told of a raised window, which a peer announces in
    /// a `MAX_DATA` frame. A connection that has carried no data has no other reason to
    /// receive one.
    async fn raised(connection: &Connection, patience: Duration) -> bool {
        for _ in 0..(patience.as_millis() / 20) {
            if connection.stats().frame_rx.max_data > 0 {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }

    #[tokio::test]
    async fn both_ends_of_a_paired_session_connection_raise_their_window() {
        let dir = tempfile::tempdir().unwrap();
        let (a_key, b_key) = (SecretKey::generate(), SecretKey::generate());
        let a_trust = TrustStore::load(dir.path().join("a.json")).unwrap();
        let b_trust = TrustStore::load(dir.path().join("b.json")).unwrap();
        a_trust
            .pair(
                DeviceId::from_bytes(*b_key.public().as_bytes()),
                &device_info(),
                1,
            )
            .unwrap();
        b_trust
            .pair(
                DeviceId::from_bytes(*a_key.public().as_bytes()),
                &device_info(),
                1,
            )
            .unwrap();
        let a = gated_endpoint(a_key, a_trust).await;
        let b = gated_endpoint(b_key, b_trust).await;

        let accepting = tokio::spawn({
            let b = b.clone();
            async move { b.accept().await.unwrap().await.unwrap() }
        });
        let address = EndpointAddr::new(b.id()).with_ip_addr(b.bound_sockets()[0]);
        let dialed = a.connect(address, ALPN_SESSION).await.unwrap();
        let accepted = accepting.await.unwrap();

        // Each end raised its own window after the check, and so told the other. It is the
        // same for the end that dialed, so it does not matter which side asks for a file.
        assert!(
            raised(&dialed, Duration::from_secs(5)).await,
            "the device that was dialed did not raise its window"
        );
        assert!(
            raised(&accepted, Duration::from_secs(5)).await,
            "the device that dialed did not raise its window"
        );
    }

    #[tokio::test]
    async fn a_pairing_connection_keeps_the_small_window_in_both_directions() {
        let dir = tempfile::tempdir().unwrap();
        let a_trust = TrustStore::load(dir.path().join("a.json")).unwrap();
        let b_trust = TrustStore::load(dir.path().join("b.json")).unwrap();
        // Nobody is paired with anybody.
        let a = gated_endpoint(SecretKey::generate(), a_trust).await;
        let b = gated_endpoint(SecretKey::generate(), b_trust).await;

        let accepting = tokio::spawn({
            let b = b.clone();
            async move { b.accept().await.unwrap().await.unwrap() }
        });
        let address = EndpointAddr::new(b.id()).with_ip_addr(b.bound_sockets()[0]);
        let dialed = a.connect(address, ALPN_PAIR).await.unwrap();
        let accepted = accepting.await.unwrap();

        assert!(
            !raised(&dialed, Duration::from_millis(400)).await,
            "a pairing connection was given the big window"
        );
        assert!(
            !raised(&accepted, Duration::from_millis(400)).await,
            "a pairing connection was given the big window"
        );
    }
}
