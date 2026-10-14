// SPDX-License-Identifier: Apache-2.0

//! Deciding who may connect and who may stay connected.
//!
//! The transport authenticates every peer by its key, but a key being genuine
//! says nothing about whether we want to talk to it. Two checks sit in front of
//! the protocol handlers:
//!
//! - [`Admission`] runs on the first packet of an incoming connection, before
//!   any cryptography, and sheds floods cheaply.
//! - [`Gate`] runs right after the handshake, before any data is read, and closes
//!   session connections from devices that are not paired.
//!
//! [`ConnectionRegistry`] remembers the open session connections, in both
//! directions, so that unpairing a device can close them at once.

use std::{
    collections::HashMap,
    fmt,
    net::IpAddr,
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
use crate::{ratelimit::RateLimiter, trust::TrustStore, util::lock};

/// Most session connections kept open to or from one paired device. A well-behaved
/// device needs one; the margin is for reconnects that overlap.
const MAX_CONNECTIONS_PER_PEER: usize = 8;

/// Handshakes from sources that have not proven their address that are let
/// through each second without asking them to. Beyond this the node asks for proof
/// first, which costs it nothing but a small reply.
const UNPROVEN_PER_SECOND: u32 = 64;

/// Connection attempts allowed from one proven source per [`SOURCE_WINDOW`].
const SOURCE_BURST: u32 = 20;
const SOURCE_WINDOW: Duration = Duration::from_secs(10);
/// Sources remembered at once. Old ones are forgotten first.
const SOURCE_KEYS: usize = 1024;

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
    pub(crate) fn new(trust: TrustStore, registry: Arc<ConnectionRegistry>) -> Self {
        Self {
            trust,
            registry,
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
            AfterHandshakeOutcome::Accept
        } else {
            Self::refuse_unpaired()
        }
    }
}

/// Where an incoming connection attempt comes from, as far as that can be told
/// before the handshake.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Source {
    Address(IpAddr),
    Relayed(EndpointId),
}

/// Decides whether an incoming connection attempt is worth a handshake.
///
/// This runs on the first packet, before any key is derived, so it has to be
/// cheap and it has to be careful about what it believes. The source address of
/// a UDP packet can be forged, so an address is only held to account once it has
/// answered a retry token, and limiting an address that has not proven itself
/// would let anyone on the internet lock a real device out by forging its address.
#[derive(Debug)]
pub(crate) struct Admission {
    unproven: Mutex<RateLimiter<()>>,
    sources: Mutex<RateLimiter<Source>>,
}

impl Admission {
    pub(crate) fn new() -> Self {
        Self {
            unproven: Mutex::new(RateLimiter::new(
                UNPROVEN_PER_SECOND,
                Duration::from_secs(1),
                1,
            )),
            sources: Mutex::new(RateLimiter::new(SOURCE_BURST, SOURCE_WINDOW, SOURCE_KEYS)),
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
            IncomingAddr::Ip(addr) => Source::Address(addr.ip()),
            // The relay has already checked that the sender holds this key.
            IncomingAddr::Relay { endpoint_id, .. } => Source::Relayed(*endpoint_id),
            // No other transport is enabled.
            _ => return IncomingFilterOutcome::Reject,
        };
        if matches!(source, Source::Address(_)) && !validated {
            // Quiet times cost nothing. When many unproven attempts arrive at once,
            // asking for a retry token makes each of them prove its address first,
            // and a forged address never sees the question.
            return if lock(&self.unproven).allow(&(), now) {
                IncomingFilterOutcome::Accept
            } else {
                IncomingFilterOutcome::Retry
            };
        }
        if lock(&self.sources).allow(&source, now) {
            IncomingFilterOutcome::Accept
        } else {
            IncomingFilterOutcome::Reject
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use iroh::{RelayUrl, SecretKey};

    use super::*;

    fn ip(last: u8) -> IncomingAddr {
        IncomingAddr::Ip(SocketAddr::from(([192, 168, 1, last], 4433)))
    }

    fn relayed(seed: u8) -> IncomingAddr {
        IncomingAddr::Relay {
            url: "https://relay.example".parse::<RelayUrl>().unwrap(),
            endpoint_id: SecretKey::from_bytes(&[seed; 32]).public(),
        }
    }

    #[test]
    fn quiet_unproven_sources_are_let_through() {
        let admission = Admission::new();
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
        let admission = Admission::new();
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
        let admission = Admission::new();
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
        let admission = Admission::new();
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
    fn relayed_senders_are_limited_by_their_key() {
        let admission = Admission::new();
        let now = Instant::now();
        for _ in 0..SOURCE_BURST {
            assert_eq!(
                admission.judge_at(&relayed(1), false, now),
                IncomingFilterOutcome::Accept
            );
        }
        assert_eq!(
            admission.judge_at(&relayed(1), false, now),
            IncomingFilterOutcome::Reject
        );
        assert_eq!(
            admission.judge_at(&relayed(2), false, now),
            IncomingFilterOutcome::Accept
        );
    }
}
