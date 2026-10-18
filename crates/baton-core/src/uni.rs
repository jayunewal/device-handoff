// SPDX-License-Identifier: Apache-2.0

//! The unidirectional streams of a session connection: who takes them.
//!
//! A paired device opens unidirectional streams to this one for the lanes of a file transfer
//! (`docs/protocol.md` section 8) and, for window streaming, for the frames of video and the
//! control streams (section 9). The first byte of a stream says which, and the next 16 name the
//! transfer or the session that it belongs to. QUIC has one limit for all of them and does not
//! know what they carry, so one task for each connection takes every stream, whatever it carries,
//! and the rules that are the same for every kind are applied here and not once for each:
//!
//! - A stream has to say what it is. It has [`Uni::start_deadline`] to send its first 17 bytes,
//!   and one that does not is stopped with 408.
//! - The peer has to be paired when the stream is taken, not only when the connection was made.
//!   Section 8 asks for it when a lane is taken.
//! - A kind that nothing here takes is stopped with 400, like a kind that nobody knows.
//! - The open streams of each kind are counted, because a stream beyond the count of its kind would
//!   take the place of a stream of another kind (ADR 0009).
//! - The limit that QUIC applies to what a peer may open is zero until the gate has seen that the
//!   peer is paired, and then it is the sum of what the kinds that are taken need
//!   ([`Sinks::quic_limit`]).
//!
//! What a stream means is for the [`Sink`] that takes its kind: which transfer or session it
//! belongs to and whether this peer is part of it, which section 8 and section 9 each say, and what
//! to do with the bytes. A sink gets the stream with its start read, and a [`Slot`] that holds the
//! stream's place in the count until the sink lets go of it.
//!
//! The task runs on both kinds of connection that a device has with a peer, the one it accepted and
//! the one it dialed. Which side opens a stream depends on which side has something to send, and a
//! device that has dialed a peer is as likely to be sent a stream on that connection as one that
//! accepted it: a viewer that was offered a window opens its control stream on the connection
//! that the source made.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use baton_proto::{DeviceId, close_code, transfer::lane::KIND_LANE};
use baton_stream::{KIND_CONTROL, KIND_VIDEO, header::PREFIX_LEN};
use iroh::endpoint::{Connection, RecvStream};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::{
    gate::{code, remote_device},
    transfer::incoming::Link,
    trust::{PairedDevice, TrustStore},
    util::{contained, lock},
};

/// The bytes that every unidirectional stream starts with: the kind, and the 16 bytes that name
/// the transfer (for a lane) or the session (for the other kinds).
pub(crate) const START_LEN: usize = PREFIX_LEN;

/// What the first byte of a unidirectional stream says that it carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the kinds of window streaming are taken when it is added"
    )
)]
pub(crate) enum Kind {
    /// A lane of a file transfer (protocol.md section 8).
    Lane,
    /// One frame of video (section 9).
    Video,
    /// A control stream (section 9).
    Control,
}

impl Kind {
    const COUNT: usize = 3;

    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            KIND_LANE => Some(Self::Lane),
            KIND_VIDEO => Some(Self::Video),
            KIND_CONTROL => Some(Self::Control),
            _ => None,
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::Lane => 0,
            Self::Video => 1,
            Self::Control => 2,
        }
    }

    /// What a stream beyond the count of its kind is stopped with. A lane or a control stream
    /// beyond it is a violation. A frame of video is not: the source cannot see when the viewer
    /// stops counting a stream that it reset, so a frame can arrive just as the count is reached,
    /// and that is no one's fault (ADR 0009).
    const fn beyond_the_count(self) -> u32 {
        match self {
            Self::Lane | Self::Control => close_code::PROTOCOL_VIOLATION,
            Self::Video => close_code::CANCELLED,
        }
    }
}

/// A stream that was accepted and whose start was read.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "the kind is read when window streaming is added")
)]
pub(crate) struct Arrival {
    /// What the stream carries.
    pub(crate) kind: Kind,
    /// The 16 bytes after the kind: the id of the transfer for a lane, and the id of the session
    /// for the other kinds. Whether a transfer or a session has this id, and whether this peer is
    /// part of it, is for the sink to say.
    pub(crate) id: [u8; START_LEN - 1],
    /// The device that opened the stream, as the list of paired devices has it.
    pub(crate) peer: PairedDevice,
    /// The connection that the stream came on. A device may keep several to one peer, and some
    /// things are counted for each (the numbers of lanes, section 8).
    pub(crate) connection: Connection,
    /// The same connection, as what is kept for each connection tells it apart: the attempt of a
    /// transfer is on one. The session made it with the connection, and uses it for the requests
    /// that come on the connection too.
    pub(crate) link: Link,
    /// The stream, read to the end of its start.
    pub(crate) recv: RecvStream,
    /// The stream's place in the count of its kind. It is given back when this is dropped, so the
    /// sink keeps it for as long as it holds the stream.
    pub(crate) slot: Slot,
}

/// What takes the streams of the kinds it is installed for.
pub(crate) trait Sink: Send + Sync + 'static {
    /// Takes over a stream.
    ///
    /// This does not wait. What takes time is done by a task of the sink's own, which keeps the
    /// arrival until it is done with it. A sink that refuses a stream stops it with the code that
    /// says why (`RecvStream::stop`, the codes of section 3), and drops the arrival.
    ///
    /// A sink lets go of the arrival, and with it the stream's place in the count, before it
    /// answers anything that ends what the stream belongs to: `Done` or `Cancelled` for the lanes
    /// of a transfer, the end of the session for a control stream. The limit that the gate
    /// announces has room for the other kinds, so QUIC no longer holds a sender back, and the
    /// next stream of the kind would find the place still taken and be stopped as a violation
    /// (section 9, Taking a stream).
    fn take(&self, arrival: Arrival);
}

/// A sink, and how many streams of its kind may be open at once on one connection.
struct Installed {
    sink: Arc<dyn Sink>,
    limit: u32,
}

/// What takes which kinds.
#[derive(Default)]
pub(crate) struct Sinks {
    by_kind: [Option<Installed>; Kind::COUNT],
}

impl Sinks {
    /// Has `sink` take the streams of `kind`, at most `limit` of them at once on a connection.
    /// A limit of zero takes none.
    pub(crate) fn install(mut self, kind: Kind, limit: u32, sink: Arc<dyn Sink>) -> Self {
        if let Some(slot) = self.by_kind.get_mut(kind.index()) {
            *slot = (limit > 0).then_some(Installed { sink, limit });
        }
        self
    }

    fn get(&self, kind: Kind) -> Option<&Installed> {
        self.by_kind.get(kind.index())?.as_ref()
    }

    /// How many unidirectional streams QUIC lets a paired device have open at once: the sum of
    /// what the kinds that are taken need, and zero when none is.
    pub(crate) fn quic_limit(&self) -> u32 {
        self.by_kind
            .iter()
            .flatten()
            .fold(0, |sum, installed| sum.saturating_add(installed.limit))
    }
}

/// How many streams of each kind are open on one connection.
#[derive(Default)]
struct Counts(Mutex<[u32; Kind::COUNT]>);

impl Counts {
    /// Takes a place for a stream of `kind`, unless `limit` of them are open already.
    fn take(self: &Arc<Self>, kind: Kind, limit: u32) -> Option<Slot> {
        let mut counts = lock(&self.0);
        let open = counts.get_mut(kind.index())?;
        if *open >= limit {
            return None;
        }
        *open += 1;
        Some(Slot {
            counts: Arc::clone(self),
            kind,
        })
    }

    #[cfg(test)]
    fn open(&self, kind: Kind) -> u32 {
        lock(&self.0).get(kind.index()).copied().unwrap_or(0)
    }
}

/// The place of an open stream in the count of its kind. Dropping it gives the place back.
pub(crate) struct Slot {
    counts: Arc<Counts>,
    kind: Kind,
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(open) = lock(&self.counts.0).get_mut(self.kind.index()) {
            *open = open.saturating_sub(1);
        }
    }
}

/// Takes the unidirectional streams of the session connections of a node.
pub(crate) struct Uni {
    sinks: Sinks,
    trust: TrustStore,
    stop: CancellationToken,
    /// How long a stream has to send its first [`START_LEN`] bytes.
    start_deadline: Duration,
}

impl Uni {
    pub(crate) fn new(
        sinks: Sinks,
        trust: &TrustStore,
        stop: &CancellationToken,
        start_deadline: Duration,
    ) -> Self {
        Self {
            sinks,
            trust: trust.clone(),
            stop: stop.clone(),
            start_deadline,
        }
    }

    /// The limit on unidirectional streams that the gate announces to a paired device, see
    /// [`Sinks::quic_limit`].
    pub(crate) fn quic_limit(&self) -> u32 {
        self.sinks.quic_limit()
    }

    /// Takes the unidirectional streams of `connection`, which is a connection to a paired
    /// device and has the link `link`, until it ends. Nothing is started when no kind is taken:
    /// QUIC lets no stream through then either.
    pub(crate) fn spawn(self: &Arc<Self>, connection: Connection, link: Link) {
        if self.quic_limit() == 0 {
            return;
        }
        let uni = Arc::clone(self);
        tokio::spawn(contained("unidirectional streams", async move {
            uni.serve(connection, link).await;
        }));
    }

    async fn serve(self: Arc<Self>, connection: Connection, link: Link) {
        let peer = remote_device(&connection);
        let counts = Arc::new(Counts::default());
        // Every stream that is open and has not sent its start yet is here. QUIC bounds how many
        // there can be.
        let mut starting = JoinSet::new();
        loop {
            tokio::select! {
                biased;
                () = self.stop.cancelled() => break,
                accepted = connection.accept_uni() => {
                    // An error means the connection ended.
                    let Ok(recv) = accepted else { break };
                    starting.spawn(Arc::clone(&self).route(
                        connection.clone(),
                        link,
                        peer,
                        Arc::clone(&counts),
                        recv,
                    ));
                }
                Some(finished) = starting.join_next(), if !starting.is_empty() => {
                    if let Err(e) = finished
                        && e.is_panic()
                    {
                        tracing::error!(peer = %peer.short(), "a unidirectional stream handler panicked");
                    }
                }
            }
        }
    }

    /// Reads the start of a stream and hands the stream to the sink of its kind, or stops it.
    async fn route(
        self: Arc<Self>,
        connection: Connection,
        link: Link,
        peer: DeviceId,
        counts: Arc<Counts>,
        mut recv: RecvStream,
    ) {
        // The list of paired devices may have changed since the connection was made. A device
        // that is unpaired has its connections closed at once, so this is for the moment between.
        let Some(paired) = self.trust.get(&peer) else {
            connection.close(code(close_code::NOT_PAIRED), b"not paired");
            return;
        };

        let mut start = [0_u8; START_LEN];
        match tokio::time::timeout(self.start_deadline, recv.read_exact(&mut start)).await {
            Ok(Ok(())) => {}
            Err(_) => {
                stop(&mut recv, close_code::TIMEOUT);
                return;
            }
            // The peer reset the stream, or the connection went, or the peer finished the stream
            // before its start was complete. None of it leaves anything to do.
            Ok(Err(_)) => return,
        }
        let [kind, id @ ..] = start;

        let Some((kind, installed)) =
            Kind::from_byte(kind).and_then(|kind| Some((kind, self.sinks.get(kind)?)))
        else {
            // A kind that nobody knows, or that this device does not take (protocol.md
            // section 9): all the same to the peer.
            stop(&mut recv, close_code::PROTOCOL_VIOLATION);
            return;
        };
        let Some(slot) = counts.take(kind, installed.limit) else {
            stop(&mut recv, kind.beyond_the_count());
            return;
        };
        installed.sink.take(Arrival {
            kind,
            id,
            peer: paired,
            connection,
            link,
            recv,
            slot,
        });
    }
}

/// Tells the peer to stop sending on a stream, with the code that says why.
pub(crate) fn stop(recv: &mut RecvStream, error: u32) {
    // A stream that has ended already needs no stopping.
    let _ = recv.stop(code(error));
}

#[cfg(test)]
mod tests {
    use baton_proto::{ALPN_SESSION, DeviceInfo, Platform};
    use iroh::{
        Endpoint, EndpointAddr, SecretKey,
        endpoint::{ConnectionError, SendStream, presets},
    };
    use tokio::sync::mpsc;

    use super::*;
    use crate::{
        gate::{ConnectionRegistry, Gate},
        testing::TestNode,
        transport,
    };

    const PATIENCE: Duration = Duration::from_secs(10);

    /// Long enough for something that should not happen to have happened.
    const QUIET: Duration = Duration::from_millis(400);

    /// The start of a stream: a kind, and an id of 16 equal bytes.
    fn start(kind: u8, id: u8) -> Vec<u8> {
        let mut bytes = vec![kind];
        bytes.extend_from_slice(&[id; START_LEN - 1]);
        bytes
    }

    /// Passes on what it is given.
    struct Collect(mpsc::UnboundedSender<Arrival>);

    impl Sink for Collect {
        fn take(&self, arrival: Arrival) {
            let _ = self.0.send(arrival);
        }
    }

    fn collector() -> (Arc<dyn Sink>, mpsc::UnboundedReceiver<Arrival>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (Arc::new(Collect(sender)), receiver)
    }

    async fn next(receiver: &mut mpsc::UnboundedReceiver<Arrival>) -> Arrival {
        tokio::time::timeout(PATIENCE, receiver.recv())
            .await
            .expect("no stream arrived")
            .expect("the sink was dropped")
    }

    async fn nothing_arrives(receiver: &mut mpsc::UnboundedReceiver<Arrival>) {
        assert!(
            tokio::time::timeout(QUIET, receiver.recv()).await.is_err(),
            "a stream arrived that should have been stopped"
        );
    }

    fn info() -> DeviceInfo {
        DeviceInfo {
            name: "Device".to_owned(),
            platform: Platform::Linux,
            app_version: "0.0.0-test".to_owned(),
        }
    }

    /// A bare endpoint on loopback behind the gate, as a node has it, with the dispatcher for
    /// the connections that it makes and accepts.
    struct Device {
        endpoint: Endpoint,
        trust: TrustStore,
        uni: Arc<Uni>,
        id: DeviceId,
        _dir: tempfile::TempDir,
    }

    impl Device {
        async fn new(sinks: Sinks, start_deadline: Duration) -> Self {
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
        async fn taking(kinds: &[(Kind, u32)]) -> (Self, Vec<mpsc::UnboundedReceiver<Arrival>>) {
            Self::taking_within(kinds, PATIENCE).await
        }

        async fn taking_within(
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

        fn pair_with(&self, other: &Self) {
            self.trust.pair(other.id, &info(), 1).unwrap();
        }
    }

    /// Pairs two devices, connects `from` to `to`, and has each end take the streams of its
    /// connection, as the session code does. Returns the connection as each end has it.
    async fn connect(from: &Device, to: &Device) -> (Connection, Connection) {
        from.pair_with(to);
        to.pair_with(from);
        let accepting = tokio::spawn({
            let endpoint = to.endpoint.clone();
            async move { endpoint.accept().await.unwrap().await.unwrap() }
        });
        let address =
            EndpointAddr::new(to.endpoint.id()).with_ip_addr(to.endpoint.bound_sockets()[0]);
        let dialed = from.endpoint.connect(address, ALPN_SESSION).await.unwrap();
        let accepted = accepting.await.unwrap();
        from.uni.spawn(dialed.clone(), Link(1));
        to.uni.spawn(accepted.clone(), Link(2));
        (dialed, accepted)
    }

    /// Opens a stream and writes `bytes` on it. The stream stays open.
    async fn open(connection: &Connection, bytes: &[u8]) -> SendStream {
        let mut send = tokio::time::timeout(PATIENCE, connection.open_uni())
            .await
            .expect("the stream could not be opened")
            .unwrap();
        send.write_all(bytes).await.unwrap();
        send
    }

    /// The code that the other end stopped `send` with.
    async fn stopped_with(send: &SendStream) -> u64 {
        tokio::time::timeout(PATIENCE, send.stopped())
            .await
            .expect("the stream was never stopped")
            .unwrap()
            .expect("the stream was finished and not stopped")
            .into_inner()
    }

    #[tokio::test]
    async fn each_stream_goes_to_the_sink_of_its_kind_with_what_follows_its_start() {
        let (b, mut taken) =
            Device::taking(&[(Kind::Lane, 2), (Kind::Video, 2), (Kind::Control, 1)]).await;
        let (a, _) = Device::taking(&[]).await;
        let (dialed, _) = connect(&a, &b).await;
        let [lanes, videos, controls] = taken.as_mut_slice() else {
            panic!("three sinks");
        };

        for (kind, id, rest, receiver) in [
            (KIND_LANE, 7, &b"lane bytes"[..], &mut *lanes),
            (KIND_VIDEO, 8, &b"a picture"[..], &mut *videos),
            (KIND_CONTROL, 9, &b"hello"[..], &mut *controls),
        ] {
            let mut send = open(&dialed, &[start(kind, id).as_slice(), rest].concat()).await;
            send.finish().unwrap();
            let mut arrival = next(receiver).await;
            assert_eq!(Kind::from_byte(kind), Some(arrival.kind));
            assert_eq!(arrival.id, [id; START_LEN - 1]);
            assert_eq!(arrival.peer.id, a.id);
            // The start is read, and the rest of the stream is the sink's.
            assert_eq!(arrival.recv.read_to_end(1024).await.unwrap(), rest);
        }
        // Each sink got its own.
        for receiver in [lanes, videos, controls] {
            nothing_arrives(receiver).await;
        }
    }

    #[tokio::test]
    async fn a_kind_that_nobody_takes_or_knows_is_stopped_with_400() {
        // Lanes only: window streaming is not installed, so its kinds are no more known than 9.
        let (b, mut taken) = Device::taking(&[(Kind::Lane, 2)]).await;
        let (a, _) = Device::taking(&[]).await;
        let (dialed, _) = connect(&a, &b).await;

        for kind in [KIND_VIDEO, KIND_CONTROL, 9] {
            let send = open(&dialed, &start(kind, 1)).await;
            assert_eq!(stopped_with(&send).await, 400, "kind {kind}");
            // A sender that is stopped lets go of the stream, which gives its place back. There
            // are only two places.
            drop(send);
        }
        // And the connection is as it was: a lane still comes through.
        let mut lane = open(&dialed, &start(KIND_LANE, 2)).await;
        lane.finish().unwrap();
        assert_eq!(next(&mut taken[0]).await.kind, Kind::Lane);
    }

    #[tokio::test]
    async fn a_stream_that_does_not_send_its_start_in_time_is_stopped_with_408() {
        let (b, mut taken) =
            Device::taking_within(&[(Kind::Lane, 2)], Duration::from_millis(200)).await;
        let (a, _) = Device::taking(&[]).await;
        let (dialed, _) = connect(&a, &b).await;

        // Five bytes of 17, and then nothing.
        let slow = open(&dialed, &start(KIND_LANE, 1)[..5]).await;
        assert_eq!(stopped_with(&slow).await, 408);
        nothing_arrives(&mut taken[0]).await;
    }

    #[tokio::test]
    async fn a_stream_that_ends_before_its_start_is_complete_is_dropped() {
        let (b, mut taken) = Device::taking(&[(Kind::Lane, 2)]).await;
        let (a, _) = Device::taking(&[]).await;
        let (dialed, _) = connect(&a, &b).await;

        let mut short = open(&dialed, &start(KIND_LANE, 1)[..5]).await;
        short.finish().unwrap();
        let mut whole = open(&dialed, &start(KIND_LANE, 2)).await;
        whole.finish().unwrap();
        // Only the whole one arrives, and the short one took no place from it.
        let arrival = next(&mut taken[0]).await;
        assert_eq!(arrival.id, [2; START_LEN - 1]);
        nothing_arrives(&mut taken[0]).await;
    }

    /// Opens `count` streams of `kind` to a device that takes `kinds`, and holds on to what it
    /// is given. Returns what the streams after the first `limit` were stopped with.
    async fn open_beyond(kinds: &[(Kind, u32)], kind: u8, count: usize) -> Vec<u64> {
        let (b, mut taken) = Device::taking(kinds).await;
        let (a, _) = Device::taking(&[]).await;
        let (dialed, _) = connect(&a, &b).await;
        let at = kinds
            .iter()
            .position(|(k, _)| Some(*k) == Kind::from_byte(kind))
            .unwrap();
        let limit = usize::try_from(kinds[at].1).unwrap();

        let mut sends = Vec::new();
        let mut held = Vec::new();
        for i in 0..count {
            let id = u8::try_from(i).unwrap();
            sends.push(open(&dialed, &start(kind, id)).await);
            if i < limit {
                held.push(next(&mut taken[at]).await);
            }
        }
        let mut codes = Vec::new();
        for send in &sends[limit..] {
            codes.push(stopped_with(send).await);
        }
        nothing_arrives(&mut taken[at]).await;
        assert_eq!(held.len(), limit);
        codes
    }

    #[tokio::test]
    async fn a_lane_beyond_the_count_of_lanes_is_a_violation() {
        // The video allowance leaves room for a lane that is beyond the lanes, which is what
        // the count is for.
        let codes = open_beyond(&[(Kind::Lane, 2), (Kind::Video, 4)], KIND_LANE, 3).await;
        assert_eq!(codes, [400]);
    }

    #[tokio::test]
    async fn a_control_stream_beyond_the_count_is_a_violation() {
        let codes = open_beyond(&[(Kind::Control, 1), (Kind::Lane, 4)], KIND_CONTROL, 2).await;
        assert_eq!(codes, [400]);
    }

    #[tokio::test]
    async fn a_frame_beyond_the_count_of_frames_is_stopped_with_499_and_no_one_is_at_fault() {
        let codes = open_beyond(&[(Kind::Video, 2), (Kind::Lane, 4)], KIND_VIDEO, 4).await;
        assert_eq!(codes, [499, 499]);
    }

    #[tokio::test]
    async fn the_place_of_a_stream_is_given_back_when_the_sink_lets_go_of_its_slot() {
        let (b, mut taken) = Device::taking(&[(Kind::Lane, 1), (Kind::Video, 3)]).await;
        let (a, _) = Device::taking(&[]).await;
        let (dialed, _) = connect(&a, &b).await;

        let _first = open(&dialed, &start(KIND_LANE, 1)).await;
        let held = next(&mut taken[0]).await;
        let second = open(&dialed, &start(KIND_LANE, 2)).await;
        assert_eq!(stopped_with(&second).await, 400);
        // The kinds are counted apart: a full set of lanes leaves the frames alone.
        let mut frame = open(&dialed, &start(KIND_VIDEO, 3)).await;
        frame.finish().unwrap();
        assert_eq!(next(&mut taken[1]).await.kind, Kind::Video);

        // The place is the slot's, whatever else of the stream the sink still holds.
        drop(held.slot);
        let mut third = open(&dialed, &start(KIND_LANE, 3)).await;
        third.finish().unwrap();
        assert_eq!(next(&mut taken[0]).await.id, [3; START_LEN - 1]);
    }

    #[tokio::test]
    async fn quic_lets_a_paired_device_open_as_many_streams_as_the_kinds_need_and_no_more() {
        let (b, mut taken) = Device::taking(&[(Kind::Lane, 2), (Kind::Video, 1)]).await;
        assert_eq!(b.uni.quic_limit(), 3);
        let (a, _) = Device::taking(&[]).await;
        let (dialed, _) = connect(&a, &b).await;

        let mut sends = Vec::new();
        for i in 0..3 {
            sends.push(open(&dialed, &start(KIND_LANE, i)).await);
        }
        // Two lanes and a third that is beyond them are three streams, which QUIC allows. A fourth
        // has to wait for a place.
        let held = vec![next(&mut taken[0]).await, next(&mut taken[0]).await];
        assert_eq!(stopped_with(&sends[2]).await, 400);
        let blocked = tokio::time::timeout(QUIET, dialed.open_uni()).await;
        assert!(blocked.is_err(), "a fourth stream was let through");
        // The streams end: the device lets go of what it holds, and the senders, which are
        // stopped, let go of theirs. Places come back, so the limit is not a leak.
        drop(held);
        drop(sends);
        let reopened = tokio::time::timeout(PATIENCE, dialed.open_uni()).await;
        assert!(reopened.is_ok(), "no place was given back");
    }

    #[tokio::test]
    async fn a_device_that_takes_nothing_lets_no_stream_through() {
        let (b, _) = Device::taking(&[]).await;
        assert_eq!(b.uni.quic_limit(), 0);
        let (a, _) = Device::taking(&[]).await;
        let (dialed, accepted) = connect(&a, &b).await;
        for connection in [&dialed, &accepted] {
            let blocked = tokio::time::timeout(QUIET, connection.open_uni()).await;
            assert!(blocked.is_err(), "a unidirectional stream was let through");
        }
    }

    #[tokio::test]
    async fn the_connection_that_a_device_dialed_takes_streams_too() {
        let (b, _) = Device::taking(&[(Kind::Lane, 1)]).await;
        let (a, mut taken) = Device::taking(&[(Kind::Control, 1)]).await;
        let (_, accepted) = connect(&a, &b).await;

        // The device that was dialed opens a stream to the one that dialed, as a viewer opens its
        // control stream to the source that made the offer.
        let mut control = open(&accepted, &start(KIND_CONTROL, 5)).await;
        control.finish().unwrap();
        let arrival = next(&mut taken[0]).await;
        assert_eq!(arrival.kind, Kind::Control);
        assert_eq!(arrival.peer.id, b.id);
    }

    #[tokio::test]
    async fn a_device_that_is_unpaired_while_connected_is_cut_off_at_its_next_stream() {
        let (b, mut taken) = Device::taking(&[(Kind::Lane, 2)]).await;
        let (a, _) = Device::taking(&[]).await;
        let (dialed, _) = connect(&a, &b).await;

        b.trust.remove(&a.id).unwrap();
        let _lane = open(&dialed, &start(KIND_LANE, 1)).await;
        let closed = tokio::time::timeout(PATIENCE, dialed.closed())
            .await
            .expect("the connection was not closed");
        match closed {
            ConnectionError::ApplicationClosed(close) => {
                assert_eq!(u64::from(close.error_code), 403);
            }
            other => panic!("closed in another way: {other:?}"),
        }
        nothing_arrives(&mut taken[0]).await;
    }

    #[tokio::test]
    async fn a_stream_is_not_taken_once_the_node_has_stopped() {
        let (b, mut taken) = Device::taking(&[(Kind::Lane, 2)]).await;
        let (a, _) = Device::taking(&[]).await;
        // The connection stays up on both ends, so that what stops is the node and not it.
        let (dialed, _accepted) = connect(&a, &b).await;
        b.uni.stop.cancel();
        // Give the loop the moment it needs to see it.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let _lane = open(&dialed, &start(KIND_LANE, 1)).await;
        nothing_arrives(&mut taken[0]).await;
    }

    /// A node on loopback that takes the streams of `kinds`, and what it was given.
    async fn node(kinds: &[(Kind, u32)]) -> (TestNode, Vec<mpsc::UnboundedReceiver<Arrival>>) {
        let mut receivers = Vec::new();
        let node = TestNode::with(|mut builder| {
            for &(kind, limit) in kinds {
                let (sink, receiver) = collector();
                builder = builder.take_streams(kind, limit, sink);
                receivers.push(receiver);
            }
            builder
        })
        .await;
        (node, receivers)
    }

    #[tokio::test]
    async fn nodes_take_streams_on_the_connection_each_of_them_made_or_accepted() {
        let (a, mut a_taken) = node(&[(Kind::Control, 2)]).await;
        let (b, mut b_taken) = node(&[(Kind::Lane, 2)]).await;
        a.pair_with(&b);

        // `a` dials `b` for a request, and then opens a lane on that connection: `b` accepted it.
        a.node.ping(&b.id).await.unwrap();
        let dialed = a.node.pooled_session(&b.id).await.unwrap().connection;
        let mut lane = open(&dialed, &[start(KIND_LANE, 1).as_slice(), b"data"].concat()).await;
        lane.finish().unwrap();
        let mut arrival = next(&mut b_taken[0]).await;
        assert_eq!(arrival.kind, Kind::Lane);
        assert_eq!(arrival.peer.id, a.id);
        assert_eq!(arrival.recv.read_to_end(16).await.unwrap(), b"data");

        // `b` answers on the same connection, which `a` dialed: a stream from the device that
        // accepted it reaches the one that made it.
        let mut control = open(&arrival.connection, &start(KIND_CONTROL, 2)).await;
        control.finish().unwrap();
        let reply = next(&mut a_taken[0]).await;
        assert_eq!(reply.kind, Kind::Control);
        assert_eq!(reply.peer.id, b.id);

        // `a` takes no lanes, so a lane that `b` opens on it anyway is stopped. The limit of `a` is
        // two streams and one is in use, so the lane reaches the dispatcher and is not held back.
        let mut stray = open(&arrival.connection, &start(KIND_LANE, 3)).await;
        assert_eq!(stopped_with(&stray).await, 400);
        stray.reset(0_u8.into()).unwrap();

        a.node.shutdown().await;
        b.node.shutdown().await;
    }

    #[test]
    fn counts_give_a_place_back_when_its_slot_is_dropped() {
        let counts = Arc::new(Counts::default());
        let first = counts.take(Kind::Lane, 2).unwrap();
        let second = counts.take(Kind::Lane, 2).unwrap();
        assert!(counts.take(Kind::Lane, 2).is_none());
        // Another kind has a count of its own.
        let frame = counts.take(Kind::Video, 1).unwrap();
        assert!(counts.take(Kind::Video, 1).is_none());
        assert_eq!(counts.open(Kind::Lane), 2);
        drop(first);
        assert_eq!(counts.open(Kind::Lane), 1);
        let third = counts.take(Kind::Lane, 2).unwrap();
        drop((second, third, frame));
        assert_eq!(counts.open(Kind::Lane), 0);
        assert_eq!(counts.open(Kind::Video), 0);
    }

    #[test]
    fn the_limit_is_the_sum_of_what_the_kinds_need() {
        let (sink, _) = collector();
        let sinks = Sinks::default();
        assert_eq!(sinks.quic_limit(), 0);
        let sinks = sinks.install(Kind::Lane, 8, Arc::clone(&sink));
        assert_eq!(sinks.quic_limit(), 8);
        let sinks = sinks.install(Kind::Video, 12, Arc::clone(&sink)).install(
            Kind::Control,
            1,
            Arc::clone(&sink),
        );
        assert_eq!(sinks.quic_limit(), 21);
        // A kind is installed once, and a limit of zero takes none.
        let sinks = sinks.install(Kind::Video, 0, Arc::clone(&sink));
        assert_eq!(sinks.quic_limit(), 9);
        assert!(sinks.get(Kind::Video).is_none());
        let sinks = sinks.install(Kind::Lane, u32::MAX, sink);
        assert_eq!(sinks.quic_limit(), u32::MAX, "the sum does not wrap");
    }

    #[test]
    fn the_first_byte_names_the_kind() {
        assert_eq!(Kind::from_byte(KIND_LANE), Some(Kind::Lane));
        assert_eq!(Kind::from_byte(KIND_VIDEO), Some(Kind::Video));
        assert_eq!(Kind::from_byte(KIND_CONTROL), Some(Kind::Control));
        for byte in (0..=u8::MAX).filter(|b| ![KIND_LANE, KIND_VIDEO, KIND_CONTROL].contains(b)) {
            assert_eq!(Kind::from_byte(byte), None, "{byte}");
        }
        // A frame that is beyond the count is no one's fault, and the others are a violation.
        assert_eq!(Kind::Video.beyond_the_count(), close_code::CANCELLED);
        assert_eq!(
            Kind::Lane.beyond_the_count(),
            close_code::PROTOCOL_VIOLATION
        );
        assert_eq!(
            Kind::Control.beyond_the_count(),
            close_code::PROTOCOL_VIOLATION
        );
    }
}
