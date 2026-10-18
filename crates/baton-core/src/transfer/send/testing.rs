// SPDX-License-Identifier: Apache-2.0

//! What the tests of the sender share.
//!
//! - A [`Direct`] wire is the real table of incoming transfers ([`Incoming`]) in memory. A request is
//!   made to the table itself, and a lane is a pipe that the table's own lane reader reads, so the
//!   engine is tested against the receiver it will meet, and the things the receiver does are not
//!   guessed. A test makes the faults of section 8 happen where the wire can: a lane the receiver
//!   stops with a code, a lane that takes no byte, a connection that goes, an answer that does not
//!   arrive.
//! - A [`Script`] wire has no receiver. A test says what each request gets and what each lane does,
//!   and so can make the receiver say anything, in any order, that a real one never would.
//! - Both keep a [`Log`] of what happened to them, in order, and a lane is told apart from another
//!   by the connection and the number.
//! - A [`Rig`] is a [`Direct`] wire with a person at the other end, a folder to receive into and a
//!   folder to send from.

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    future::Future,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use baton_proto::{
    Platform,
    id::{DeviceId, TransferId},
    message::{DeclineReason, ErrorCode},
    transfer::{
        Accepted, Cancelled, Declined, Done, Failure, Offer, TransferReply, TransferRequest,
    },
};
use tempfile::TempDir;
use tokio::{
    io::{AsyncWriteExt, DuplexStream, duplex},
    sync::Notify,
    time::timeout,
};
use tokio_util::sync::CancellationToken;

use super::{
    engine::{self, Sending},
    outcome::Outcome,
    policy::Policy,
    selection::Selection,
    wire::{LaneError, LaneOut, Link, Reply, Wire, WireError},
};
use crate::{
    transfer::{
        destination::Destination,
        incoming::{self, Ending, FinishError, Incoming, TransferHandler, Verdict},
        receive::LaneEnd,
        testing::destination,
    },
    trust::PairedDevice,
    util::{BoxFuture, lock},
};

// What happened on a wire.

/// What a request is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Kind {
    Offer,
    Finish,
    Cancel,
}

impl Kind {
    fn of(request: &TransferRequest) -> Self {
        match request {
            TransferRequest::Offer(_) => Self::Offer,
            TransferRequest::Finish(_) => Self::Finish,
            TransferRequest::Cancel(_) => Self::Cancel,
        }
    }
}

/// How a lane was closed by the sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Closed {
    /// It was ended after its last byte.
    Finished,
    /// It was reset with this code.
    Reset(u32),
    /// It was let go of without being ended or reset. A stream that is dropped is finished, and
    /// looks complete to the receiver, so a lane that comes to this is a fault of the sender.
    Dropped,
}

/// One thing that happened on a wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Event {
    /// Connection number `n`, counted from 1, was made.
    Connect(usize),
    /// Connection number `n` could not be made.
    ConnectFailed(usize),
    /// A request was made on connection `n`.
    Request(usize, Kind),
    /// Lane `lane` was opened on connection `n`.
    Open(usize, u8),
    /// A lane was closed.
    Close(usize, u8, Closed),
}

/// What happened on a wire, in order, with the time of each.
#[derive(Debug, Clone, Default)]
pub(super) struct Log(Arc<Mutex<Vec<(Instant, Event)>>>);

impl Log {
    fn push(&self, event: Event) {
        lock(&self.0).push((Instant::now(), event));
    }

    pub(super) fn events(&self) -> Vec<Event> {
        lock(&self.0)
            .iter()
            .map(|(_, event)| event.clone())
            .collect()
    }

    /// When each event happened.
    pub(super) fn times(&self) -> Vec<(Instant, Event)> {
        lock(&self.0).clone()
    }

    /// How many connections were made.
    pub(super) fn connections(&self) -> usize {
        self.count(|event| matches!(event, Event::Connect(_)))
    }

    /// How many connections were asked for and could not be made.
    pub(super) fn refused_connections(&self) -> usize {
        self.count(|event| matches!(event, Event::ConnectFailed(_)))
    }

    /// How many requests of this kind were made, on any connection.
    pub(super) fn requests(&self, kind: Kind) -> usize {
        self.count(|event| matches!(event, Event::Request(_, k) if *k == kind))
    }

    /// The lanes that were opened, in order, as (connection, lane).
    pub(super) fn opened(&self) -> Vec<(usize, u8)> {
        lock(&self.0)
            .iter()
            .filter_map(|(_, event)| match event {
                Event::Open(conn, lane) => Some((*conn, *lane)),
                _ => None,
            })
            .collect()
    }

    /// How the lanes were closed, in order, as (connection, lane, how).
    pub(super) fn closed(&self) -> Vec<(usize, u8, Closed)> {
        lock(&self.0)
            .iter()
            .filter_map(|(_, event)| match event {
                Event::Close(conn, lane, how) => Some((*conn, *lane, *how)),
                _ => None,
            })
            .collect()
    }

    /// The codes that lanes were reset with.
    pub(super) fn resets(&self) -> Vec<u32> {
        self.closed()
            .into_iter()
            .filter_map(|(_, _, how)| match how {
                Closed::Reset(code) => Some(code),
                _ => None,
            })
            .collect()
    }

    fn count(&self, matching: impl Fn(&Event) -> bool) -> usize {
        lock(&self.0)
            .iter()
            .filter(|(_, event)| matching(event))
            .count()
    }
}

// Waiting for the test.

/// A door that a test opens when it wants what waits at it to go on.
#[derive(Debug, Clone, Default)]
pub(super) struct Gate(Arc<GateState>);

#[derive(Debug, Default)]
struct GateState {
    open: AtomicBool,
    opened: Notify,
}

impl Gate {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn open(&self) {
        self.0.open.store(true, Ordering::SeqCst);
        self.0.opened.notify_waiters();
    }

    pub(super) fn is_open(&self) -> bool {
        self.0.open.load(Ordering::SeqCst)
    }

    /// Waits until the gate is open.
    pub(super) async fn passed(&self) {
        loop {
            let opened = self.0.opened.notified();
            if self.is_open() {
                return;
            }
            opened.await;
        }
    }
}

// Lanes that record what they are given.

/// How a [`Sink`] answers.
#[derive(Debug, Clone)]
pub(super) enum Take {
    /// It takes everything.
    All,
    /// It takes `after` bytes, and then is stopped with `code`.
    StopAfter { after: u64, code: u32 },
    /// It takes `after` bytes, and then no write comes back.
    HangAfter { after: u64 },
    /// It takes `after` bytes, and then the stream fails.
    LoseAfter { after: u64 },
    /// It takes no byte and says so.
    Nothing,
    /// It takes everything, and is stopped with `code` when it is ended.
    StopAtEnd { code: u32 },
    /// It takes `after` bytes, and then waits for the gate to open.
    GateAfter { after: u64, gate: Gate },
    /// The lane cannot be opened: the connection fails.
    OpenFails,
    /// The lane is asked for, and never comes.
    OpenHangs,
}

/// What a [`Sink`] took, and how it was closed.
#[derive(Debug, Default)]
pub(super) struct Taken {
    bytes: Mutex<Vec<u8>>,
    closed: Mutex<Option<Closed>>,
}

impl Taken {
    pub(super) fn bytes(&self) -> Vec<u8> {
        lock(&self.bytes).clone()
    }

    pub(super) fn len(&self) -> usize {
        lock(&self.bytes).len()
    }

    pub(super) fn closed(&self) -> Option<Closed> {
        *lock(&self.closed)
    }
}

/// The most that a write takes. A write that takes less than it is given is what a stream does, and
/// the lane has to go on from where it was.
const MOST_PER_WRITE: usize = 48 * 1024;

/// A lane that takes what it is given and keeps it.
pub(super) struct Sink {
    taken: Arc<Taken>,
    take: Take,
    count: u64,
    log: Log,
    conn: usize,
    number: u8,
}

impl Sink {
    fn close(&self, how: Closed) {
        let mut closed = lock(&self.taken.closed);
        if closed.is_none() {
            *closed = Some(how);
            self.log.push(Event::Close(self.conn, self.number, how));
        }
    }
}

impl LaneOut for Sink {
    fn write<'a>(&'a mut self, bytes: &'a [u8]) -> BoxFuture<'a, Result<usize, LaneError>> {
        Box::pin(async move {
            loop {
                let limit = match &self.take {
                    Take::All | Take::StopAtEnd { .. } | Take::OpenFails | Take::OpenHangs => {
                        u64::MAX
                    }
                    Take::Nothing => return Ok(0),
                    Take::StopAfter { after, .. }
                    | Take::HangAfter { after }
                    | Take::LoseAfter { after }
                    | Take::GateAfter { after, .. } => *after,
                };
                if self.count < limit {
                    let room = usize::try_from(limit - self.count).unwrap_or(usize::MAX);
                    let n = bytes.len().min(MOST_PER_WRITE).min(room);
                    lock(&self.taken.bytes).extend_from_slice(&bytes[..n]);
                    self.count += n as u64;
                    return Ok(n);
                }
                match self.take.clone() {
                    Take::StopAfter { code, .. } => return Err(LaneError::Stopped(code)),
                    Take::HangAfter { .. } => std::future::pending::<()>().await,
                    Take::LoseAfter { .. } => return Err(LaneError::Lost),
                    Take::GateAfter { gate, .. } => {
                        gate.passed().await;
                        self.take = Take::All;
                    }
                    Take::All
                    | Take::StopAtEnd { .. }
                    | Take::Nothing
                    | Take::OpenFails
                    | Take::OpenHangs => {}
                }
            }
        })
    }

    fn finish(&mut self) -> Result<(), LaneError> {
        match &self.take {
            Take::StopAtEnd { code } => Err(LaneError::Stopped(*code)),
            Take::StopAfter { after, code } if self.count >= *after => {
                Err(LaneError::Stopped(*code))
            }
            Take::LoseAfter { after } if self.count >= *after => Err(LaneError::Lost),
            _ => {
                self.close(Closed::Finished);
                Ok(())
            }
        }
    }

    fn reset(&mut self, code: u32) {
        self.close(Closed::Reset(code));
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        self.close(Closed::Dropped);
    }
}

// A wire that a test scripts.

/// A request, as the script sees it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Call {
    /// The connection it is made on, counted from 1.
    pub(super) conn: usize,
    pub(super) kind: Kind,
    /// How many requests of this kind were made on the connection before this one.
    pub(super) nth: usize,
}

/// What a scripted request gets.
#[derive(Debug, Clone)]
pub(super) enum Step {
    Reply(Reply),
    /// No reply comes, and the request times out at once.
    Silent,
    /// No reply comes, and the request waits until it is given up.
    Never,
    /// The connection goes.
    Lost,
    /// The connection is ended in a way that retrying does not change.
    Refused(u32),
}

/// What a scripted connection does when it is asked for.
#[derive(Debug, Clone, Copy)]
pub(super) enum Connect {
    Ok,
    Lost,
    TimedOut,
    Refused(u32),
}

pub(super) fn accepted(id: TransferId, lanes: u8) -> Step {
    Step::Reply(Reply::Transfer(TransferReply::Accepted(Accepted {
        id,
        lanes,
    })))
}

pub(super) fn declined(id: TransferId, reason: DeclineReason) -> Step {
    Step::Reply(Reply::Transfer(TransferReply::Declined(Declined {
        id,
        reason,
    })))
}

pub(super) fn done(id: TransferId, failed: Vec<Failure>) -> Step {
    Step::Reply(Reply::Transfer(TransferReply::Done(Done {
        id,
        failed,
        more_failed: 0,
    })))
}

pub(super) fn cancelled(id: TransferId) -> Step {
    Step::Reply(Reply::Transfer(TransferReply::Cancelled(Cancelled { id })))
}

pub(super) fn error(code: ErrorCode) -> Step {
    Step::Reply(Reply::Error(code))
}

type Reply_ = Box<dyn Fn(&Call) -> Step + Send + Sync>;
type Takes = Box<dyn Fn(usize, u8) -> Take + Send + Sync>;
type Connects = Box<dyn Fn(usize) -> Connect + Send + Sync>;

struct ScriptState {
    log: Log,
    connections: AtomicUsize,
    reply: Reply_,
    take: Takes,
    connect: Connects,
    lanes: Mutex<Vec<(usize, u8, Arc<Taken>)>>,
}

/// A wire that has no receiver: a test says what each request gets, and what each lane does.
pub(super) struct Script(Arc<ScriptState>);

impl Script {
    /// A wire that answers every request as `reply` says, whose lanes take everything, and whose
    /// connections are made.
    pub(super) fn new(reply: impl Fn(&Call) -> Step + Send + Sync + 'static) -> Self {
        Self(Arc::new(ScriptState {
            log: Log::default(),
            connections: AtomicUsize::new(0),
            reply: Box::new(reply),
            take: Box::new(|_, _| Take::All),
            connect: Box::new(|_| Connect::Ok),
            lanes: Mutex::new(Vec::new()),
        }))
    }

    /// What each lane does, given its connection and its number.
    pub(super) fn lanes(
        mut self,
        take: impl Fn(usize, u8) -> Take + Send + Sync + 'static,
    ) -> Self {
        Arc::get_mut(&mut self.0)
            .expect("the script is not in use yet")
            .take = Box::new(take);
        self
    }

    /// What each connection does, given its number.
    pub(super) fn connects(
        mut self,
        connect: impl Fn(usize) -> Connect + Send + Sync + 'static,
    ) -> Self {
        Arc::get_mut(&mut self.0)
            .expect("the script is not in use yet")
            .connect = Box::new(connect);
        self
    }

    pub(super) fn log(&self) -> Log {
        self.0.log.clone()
    }

    /// What lane `lane` of connection `conn` was given and how it was closed, if it was opened.
    pub(super) fn lane(&self, conn: usize, lane: u8) -> Option<Arc<Taken>> {
        lock(&self.0.lanes)
            .iter()
            .find(|(c, l, _)| *c == conn && *l == lane)
            .map(|(_, _, taken)| Arc::clone(taken))
    }
}

impl Wire for Script {
    fn connect(&self) -> BoxFuture<'_, Result<Box<dyn Link>, WireError>> {
        Box::pin(async move {
            let state = &self.0;
            let number = state.connections.fetch_add(1, Ordering::SeqCst) + 1;
            match (state.connect)(number) {
                Connect::Ok => {
                    state.log.push(Event::Connect(number));
                    Ok(Box::new(ScriptLink {
                        state: Arc::clone(state),
                        number,
                        calls: Mutex::new(HashMap::new()),
                    }) as Box<dyn Link>)
                }
                failed => {
                    state.log.push(Event::ConnectFailed(number));
                    Err(match failed {
                        Connect::Lost => WireError::Lost,
                        Connect::TimedOut => WireError::TimedOut,
                        Connect::Refused(code) => WireError::Refused(code),
                        Connect::Ok => unreachable!("handled above"),
                    })
                }
            }
        })
    }
}

struct ScriptLink {
    state: Arc<ScriptState>,
    number: usize,
    calls: Mutex<HashMap<Kind, usize>>,
}

impl Link for ScriptLink {
    fn request(
        &self,
        request: TransferRequest,
        _deadline: Duration,
    ) -> BoxFuture<'_, Result<Reply, WireError>> {
        Box::pin(async move {
            let kind = Kind::of(&request);
            let nth = {
                let mut calls = lock(&self.calls);
                let count = calls.entry(kind).or_insert(0);
                *count += 1;
                *count - 1
            };
            self.state.log.push(Event::Request(self.number, kind));
            let call = Call {
                conn: self.number,
                kind,
                nth,
            };
            match (self.state.reply)(&call) {
                Step::Reply(reply) => Ok(reply),
                Step::Silent => Err(WireError::TimedOut),
                Step::Never => std::future::pending().await,
                Step::Lost => Err(WireError::Lost),
                Step::Refused(code) => Err(WireError::Refused(code)),
            }
        })
    }

    fn open_lane(
        &self,
        _id: TransferId,
        number: u8,
    ) -> BoxFuture<'_, Result<Box<dyn LaneOut>, WireError>> {
        Box::pin(async move {
            let take = (self.state.take)(self.number, number);
            match take {
                Take::OpenFails => return Err(WireError::Lost),
                Take::OpenHangs => std::future::pending().await,
                _ => {}
            }
            self.state.log.push(Event::Open(self.number, number));
            let taken = Arc::new(Taken::default());
            lock(&self.state.lanes).push((self.number, number, Arc::clone(&taken)));
            Ok(Box::new(Sink {
                taken,
                take,
                count: 0,
                log: self.state.log.clone(),
                conn: self.number,
                number,
            }) as Box<dyn LaneOut>)
        })
    }
}

// The receiver, in memory.

/// A write to a lane, as a fault sees it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Put {
    pub(super) conn: usize,
    pub(super) lane: u8,
    /// How many bytes this lane has taken so far.
    pub(super) on_lane: u64,
    /// How many bytes all the lanes of the connection have taken so far.
    pub(super) on_conn: u64,
}

/// What a fault does to a write.
#[derive(Debug, Clone)]
pub(super) enum Fault {
    /// The write goes through.
    Pass,
    /// The write waits for ever.
    Stall,
    /// The receiver stops the lane with this code.
    Stop(u32),
    /// The connection goes.
    Cut,
    /// The write waits until the gate is open, and is asked about again.
    Wait(Gate),
}

/// What a fault does to a request.
#[derive(Debug, Clone)]
pub(super) enum Fate {
    /// The request is handled as usual.
    Pass,
    /// It is handled, and the reply never arrives: the request times out.
    Swallow,
    /// It is handled, and the connection goes before the reply arrives.
    LoseReply,
    /// It is not handled: the connection goes.
    Lose,
    /// It waits until the gate is open, and is handled as usual then.
    After(Gate),
}

type OnPut = Box<dyn Fn(&Put) -> Fault + Send + Sync>;
type OnCall = Box<dyn Fn(&Call) -> Fate + Send + Sync>;
type OnConnect = Box<dyn Fn(usize) -> Option<WireError> + Send + Sync>;

struct DirectState {
    incoming: Arc<Incoming>,
    sender: PairedDevice,
    log: Log,
    connections: AtomicUsize,
    /// How much a lane holds before a write has to wait for the receiver.
    window: usize,
    /// What all the lanes of all the connections have taken.
    taken: AtomicU64,
    on_put: Mutex<Option<OnPut>>,
    on_call: Mutex<Option<OnCall>>,
    on_connect: Mutex<Option<OnConnect>>,
}

/// A wire to a real table of incoming transfers, in memory.
pub(super) struct Direct(Arc<DirectState>);

/// The device that sends, as the receiver knows it.
pub(super) fn sender_device() -> PairedDevice {
    PairedDevice {
        id: DeviceId::from_bytes([7; DeviceId::LEN]),
        name: "the sender".into(),
        local_name: None,
        name_changed: false,
        platform: Platform::Linux,
        added_at: 0,
        provenance: crate::Provenance::Paired,
        last_seen_at: None,
        relay: None,
        addrs: Vec::new(),
        policy: crate::DevicePolicy::default(),
    }
}

impl Direct {
    pub(super) fn new(incoming: Arc<Incoming>) -> Self {
        Self(Arc::new(DirectState {
            incoming,
            sender: sender_device(),
            log: Log::default(),
            connections: AtomicUsize::new(0),
            window: 64 * 1024,
            taken: AtomicU64::new(0),
            on_put: Mutex::new(None),
            on_call: Mutex::new(None),
            on_connect: Mutex::new(None),
        }))
    }

    pub(super) fn log(&self) -> Log {
        self.0.log.clone()
    }

    /// What all the lanes of all the connections have taken so far.
    pub(super) fn bytes(&self) -> u64 {
        self.0.taken.load(Ordering::SeqCst)
    }

    /// From now on `fault` decides what happens to each write to a lane.
    pub(super) fn on_put(&self, fault: impl Fn(&Put) -> Fault + Send + Sync + 'static) {
        *lock(&self.0.on_put) = Some(Box::new(fault));
    }

    /// From now on `fate` decides what happens to each request.
    pub(super) fn on_call(&self, fate: impl Fn(&Call) -> Fate + Send + Sync + 'static) {
        *lock(&self.0.on_call) = Some(Box::new(fate));
    }

    /// From now on `fails` says which connections cannot be made, and how.
    pub(super) fn on_connect(
        &self,
        fails: impl Fn(usize) -> Option<WireError> + Send + Sync + 'static,
    ) {
        *lock(&self.0.on_connect) = Some(Box::new(fails));
    }
}

impl Wire for Direct {
    fn connect(&self) -> BoxFuture<'_, Result<Box<dyn Link>, WireError>> {
        Box::pin(async move {
            let state = &self.0;
            let number = state.connections.fetch_add(1, Ordering::SeqCst) + 1;
            let failure = lock(&state.on_connect)
                .as_ref()
                .and_then(|fails| fails(number));
            if let Some(error) = failure {
                state.log.push(Event::ConnectFailed(number));
                return Err(error);
            }
            state.log.push(Event::Connect(number));
            Ok(Box::new(DirectLink(Arc::new(Conn {
                state: Arc::clone(state),
                number,
                name: incoming::Link::mint(),
                dead: CancellationToken::new(),
                bytes: AtomicU64::new(0),
                calls: Mutex::new(HashMap::new()),
            }))) as Box<dyn Link>)
        })
    }
}

/// One connection of a [`Direct`] wire.
struct Conn {
    state: Arc<DirectState>,
    number: usize,
    /// The connection as the receiver's table knows it.
    name: incoming::Link,
    /// Cancelled when the connection goes.
    dead: CancellationToken,
    /// What all the lanes of the connection have taken.
    bytes: AtomicU64,
    calls: Mutex<HashMap<Kind, usize>>,
}

impl Conn {
    fn fault(&self, lane: u8, on_lane: u64) -> Fault {
        let put = Put {
            conn: self.number,
            lane,
            on_lane,
            on_conn: self.bytes.load(Ordering::SeqCst),
        };
        lock(&self.state.on_put)
            .as_ref()
            .map_or(Fault::Pass, |fault| fault(&put))
    }

    fn fate(&self, call: &Call) -> Fate {
        lock(&self.state.on_call)
            .as_ref()
            .map_or(Fate::Pass, |fate| fate(call))
    }
}

struct DirectLink(Arc<Conn>);

impl Link for DirectLink {
    fn request(
        &self,
        request: TransferRequest,
        deadline: Duration,
    ) -> BoxFuture<'_, Result<Reply, WireError>> {
        let conn = &self.0;
        Box::pin(async move {
            if conn.dead.is_cancelled() {
                return Err(WireError::Lost);
            }
            let kind = Kind::of(&request);
            let nth = {
                let mut calls = lock(&conn.calls);
                let count = calls.entry(kind).or_insert(0);
                *count += 1;
                *count - 1
            };
            conn.state.log.push(Event::Request(conn.number, kind));
            let fate = conn.fate(&Call {
                conn: conn.number,
                kind,
                nth,
            });
            if matches!(fate, Fate::Lose) {
                conn.dead.cancel();
                return Err(WireError::Lost);
            }
            if let Fate::After(gate) = &fate {
                tokio::select! {
                    biased;
                    () = conn.dead.cancelled() => return Err(WireError::Lost),
                    () = gate.passed() => {}
                }
            }
            let sender = &conn.state.sender;
            let incoming = &conn.state.incoming;
            let handled = async {
                match request {
                    TransferRequest::Offer(offer) => {
                        Reply::Transfer(incoming.offer(sender, conn.name, offer).await)
                    }
                    TransferRequest::Finish(finish) => {
                        match incoming.finish(&sender.id, &finish).await {
                            Ok(done) => Reply::Transfer(TransferReply::Done(done)),
                            Err(FinishError::NotFound) => Reply::Error(ErrorCode::NotFound),
                            // The session ends the request without an answer.
                            Err(FinishError::TimedOut) => std::future::pending().await,
                        }
                    }
                    TransferRequest::Cancel(cancel) => {
                        incoming.cancel(&sender.id, cancel.id).await;
                        Reply::Transfer(TransferReply::Cancelled(Cancelled { id: cancel.id }))
                    }
                }
            };
            let reply = tokio::select! {
                biased;
                () = conn.dead.cancelled() => return Err(WireError::Lost),
                reply = timeout(deadline, handled) => match reply {
                    Ok(reply) => reply,
                    Err(_) => return Err(WireError::TimedOut),
                },
            };
            match fate {
                Fate::Swallow => Err(WireError::TimedOut),
                Fate::LoseReply => {
                    conn.dead.cancel();
                    Err(WireError::Lost)
                }
                Fate::Pass | Fate::Lose | Fate::After(_) => Ok(reply),
            }
        })
    }

    fn open_lane(
        &self,
        id: TransferId,
        number: u8,
    ) -> BoxFuture<'_, Result<Box<dyn LaneOut>, WireError>> {
        let conn = Arc::clone(&self.0);
        Box::pin(async move {
            if conn.dead.is_cancelled() {
                return Err(WireError::Lost);
            }
            conn.state.log.push(Event::Open(conn.number, number));
            let (tx, mut rx) = duplex(conn.state.window);
            let lane = Arc::new(LaneState::default());
            let sender = &conn.state.sender;
            match conn.state.incoming.lane(&sender.id, id, conn.name) {
                // The session stops a lane that belongs to nothing with 499.
                None => lane.stop(close_cancelled()),
                Some((attempt, guard)) => {
                    let dead = conn.dead.clone();
                    let lane = Arc::clone(&lane);
                    tokio::spawn(async move {
                        let end = tokio::select! {
                            biased;
                            () = dead.cancelled() => None,
                            () = lane.killed.cancelled() => None,
                            end = attempt.lane(number, &mut rx) => Some(end),
                        };
                        if let Some(LaneEnd::Stopped(code)) = end {
                            lane.stop(code);
                        }
                        drop(rx);
                        drop(guard);
                    });
                }
            }
            Ok(Box::new(DirectLane {
                conn,
                number,
                tx: Some(tx),
                lane,
                count: 0,
                closed: false,
            }) as Box<dyn LaneOut>)
        })
    }
}

fn close_cancelled() -> u32 {
    baton_proto::close_code::CANCELLED
}

/// What the receiver and the sender both know of a lane.
#[derive(Debug, Default)]
struct LaneState {
    /// The code the receiver stopped the lane with.
    stopped: Mutex<Option<u32>>,
    /// Cancelled when the lane stops being read: the sender reset it, or a fault stopped it.
    killed: CancellationToken,
}

impl LaneState {
    fn stop(&self, code: u32) {
        lock(&self.stopped).get_or_insert(code);
    }

    fn stopped(&self) -> Option<u32> {
        *lock(&self.stopped)
    }
}

/// The sender's end of a lane of a [`Direct`] wire.
struct DirectLane {
    conn: Arc<Conn>,
    number: u8,
    tx: Option<DuplexStream>,
    lane: Arc<LaneState>,
    count: u64,
    closed: bool,
}

impl DirectLane {
    fn close(&mut self, how: Closed) {
        if !self.closed {
            self.closed = true;
            self.conn
                .state
                .log
                .push(Event::Close(self.conn.number, self.number, how));
        }
    }
}

impl LaneOut for DirectLane {
    fn write<'a>(&'a mut self, bytes: &'a [u8]) -> BoxFuture<'a, Result<usize, LaneError>> {
        Box::pin(async move {
            loop {
                if let Some(code) = self.lane.stopped() {
                    return Err(LaneError::Stopped(code));
                }
                if self.conn.dead.is_cancelled() {
                    return Err(LaneError::Lost);
                }
                match self.conn.fault(self.number, self.count) {
                    Fault::Pass => break,
                    Fault::Stall => std::future::pending::<()>().await,
                    Fault::Stop(code) => {
                        self.lane.stop(code);
                        self.lane.killed.cancel();
                        return Err(LaneError::Stopped(code));
                    }
                    Fault::Cut => {
                        self.conn.dead.cancel();
                        return Err(LaneError::Lost);
                    }
                    Fault::Wait(gate) => gate.passed().await,
                }
            }
            let Some(tx) = self.tx.as_mut() else {
                return Err(LaneError::Lost);
            };
            let wrote = tokio::select! {
                biased;
                () = self.conn.dead.cancelled() => return Err(LaneError::Lost),
                wrote = tx.write(bytes) => wrote,
            };
            match wrote {
                Ok(taken) => {
                    let taken_now = taken as u64;
                    self.count += taken_now;
                    self.conn.bytes.fetch_add(taken_now, Ordering::SeqCst);
                    self.conn.state.taken.fetch_add(taken_now, Ordering::SeqCst);
                    Ok(taken)
                }
                Err(_) => Err(match self.lane.stopped() {
                    Some(code) => LaneError::Stopped(code),
                    None => LaneError::Lost,
                }),
            }
        })
    }

    fn finish(&mut self) -> Result<(), LaneError> {
        if let Some(code) = self.lane.stopped() {
            return Err(LaneError::Stopped(code));
        }
        if self.conn.dead.is_cancelled() {
            return Err(LaneError::Lost);
        }
        // Closing the pipe is the end of the stream: what was written is still read.
        self.tx = None;
        self.close(Closed::Finished);
        Ok(())
    }

    fn reset(&mut self, code: u32) {
        self.lane.killed.cancel();
        self.tx = None;
        self.close(Closed::Reset(code));
    }
}

impl Drop for DirectLane {
    fn drop(&mut self) {
        self.close(Closed::Dropped);
    }
}

// The person at the other end.

/// The application on the receiving device, played by the test. It puts what it receives in a folder
/// of its own, and keeps how each transfer ended.
pub(super) struct Person {
    destination: Destination,
    verdict: Mutex<Verdict>,
    /// A gate that holds the answer back, when a test wants the person to take their time.
    deliberation: Mutex<Option<Gate>>,
    asked: AtomicUsize,
    withdrawn: AtomicUsize,
    endings: Mutex<Vec<(TransferId, Ending)>>,
}

/// Counts a question that was taken back before it was answered.
struct Withdrawn<'a> {
    count: &'a AtomicUsize,
    answered: bool,
}

impl Drop for Withdrawn<'_> {
    fn drop(&mut self) {
        if !self.answered {
            self.count.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl Person {
    fn new(destination: Destination) -> Self {
        Self {
            destination,
            verdict: Mutex::new(Verdict::Accept),
            deliberation: Mutex::new(None),
            asked: AtomicUsize::new(0),
            withdrawn: AtomicUsize::new(0),
            endings: Mutex::new(Vec::new()),
        }
    }

    /// What the person says from now on.
    pub(super) fn says(&self, verdict: Verdict) {
        *lock(&self.verdict) = verdict;
    }

    /// From now on the person answers when the gate is opened, and not before.
    pub(super) fn takes_their_time(&self) -> Gate {
        let gate = Gate::new();
        *lock(&self.deliberation) = Some(gate.clone());
        gate
    }

    /// How many times the person was asked.
    pub(super) fn questions(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }

    /// How many questions were taken back before they were answered.
    pub(super) fn withdrawn(&self) -> usize {
        self.withdrawn.load(Ordering::SeqCst)
    }

    /// How the transfers that were accepted have ended.
    pub(super) fn endings(&self) -> Vec<(TransferId, Ending)> {
        lock(&self.endings).clone()
    }
}

impl TransferHandler for Person {
    fn destination(&self) -> Option<Destination> {
        Some(self.destination.clone())
    }

    fn offered<'a>(&'a self, _from: &'a PairedDevice, _offer: &'a Offer) -> BoxFuture<'a, Verdict> {
        Box::pin(async move {
            self.asked.fetch_add(1, Ordering::SeqCst);
            let mut question = Withdrawn {
                count: &self.withdrawn,
                answered: false,
            };
            let gate = lock(&self.deliberation).clone();
            if let Some(gate) = gate {
                gate.passed().await;
            }
            question.answered = true;
            *lock(&self.verdict)
        })
    }

    fn ended(&self, _from: &PairedDevice, id: TransferId, ending: Ending) {
        lock(&self.endings).push((id, ending));
    }
}

// A whole rig, and the files in it.

/// A folder of files to send.
pub(super) struct Folder {
    root: PathBuf,
    _dir: TempDir,
}

impl Folder {
    pub(super) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        Self {
            root: dir.path().canonicalize().unwrap(),
            _dir: dir,
        }
    }

    /// Makes the file `path` with `content`, and the folders on the way.
    pub(super) fn make(&self, path: &str, content: &[u8]) -> PathBuf {
        let at = self.root.join(path);
        fs::create_dir_all(at.parent().unwrap()).unwrap();
        fs::write(&at, content).unwrap();
        at
    }

    /// Makes the empty folder `path`.
    pub(super) fn make_folder(&self, path: &str) -> PathBuf {
        let at = self.root.join(path);
        fs::create_dir_all(&at).unwrap();
        at
    }

    /// What the walk makes of the items `names`.
    pub(super) fn select(&self, names: &[&str]) -> Arc<Selection> {
        let chosen: Vec<PathBuf> = names.iter().map(|name| self.root.join(name)).collect();
        Arc::new(Selection::walk(&chosen).unwrap())
    }
}

/// A [`Direct`] wire with a person at the other end, a folder that the person receives into and a
/// folder to send from.
pub(super) struct Rig {
    pub(super) person: Arc<Person>,
    pub(super) incoming: Arc<Incoming>,
    pub(super) wire: Arc<Direct>,
    /// Where the receiver puts what it receives.
    pub(super) received: PathBuf,
    /// The files to send.
    pub(super) files: Folder,
    next: AtomicU8,
    _received: TempDir,
}

impl Rig {
    pub(super) fn new() -> Self {
        let (received_dir, destination) = destination();
        let person = Arc::new(Person::new(destination));
        let incoming = Arc::new(Incoming::new(
            Arc::clone(&person) as Arc<dyn TransferHandler>,
            |_| true,
        ));
        let wire = Arc::new(Direct::new(Arc::clone(&incoming)));
        Self {
            person,
            incoming,
            wire,
            received: received_dir.path().to_path_buf(),
            files: Folder::new(),
            next: AtomicU8::new(1),
            _received: received_dir,
        }
    }

    /// An id that no other transfer of this rig has.
    pub(super) fn id(&self) -> TransferId {
        TransferId::from_bytes([self.next.fetch_add(1, Ordering::SeqCst); TransferId::LEN])
    }

    /// Starts sending `selection` with `policy`.
    pub(super) fn start(
        &self,
        selection: &Arc<Selection>,
        policy: Policy,
        id: TransferId,
    ) -> Sending {
        let wire: Arc<dyn Wire> = Arc::clone(&self.wire) as Arc<dyn Wire>;
        engine::start(wire, Arc::clone(selection), policy, id).unwrap()
    }
}

/// Everything under `root` as the relative path of each entry and its content (none for a folder),
/// except what the receiver keeps for itself.
pub(super) fn tree(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    fn visit(dir: &Path, prefix: &str, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".baton") {
                continue;
            }
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if entry.file_type().unwrap().is_dir() {
                out.insert(relative.clone(), None);
                visit(&entry.path(), &relative, out);
            } else {
                out.insert(relative, Some(fs::read(entry.path()).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, "", &mut out);
    out
}

/// How many entries the staging folders of transfers that are going hold in `root`.
pub(super) fn held(root: &Path) -> usize {
    fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".baton-incoming-")
        })
        .map(|holder| fs::read_dir(holder.path()).unwrap().count())
        .sum()
}

/// The numbers a test of the engine works to when nothing in it is meant to be slow: the waits are
/// long enough for a busy machine, the pauses between attempts are short.
pub(super) fn steady() -> Policy {
    Policy {
        offer_wait: Duration::from_secs(20),
        finish_wait: Duration::from_secs(20),
        probe_wait: Duration::from_secs(5),
        cancel_wait: Duration::from_secs(5),
        stall: Duration::from_secs(20),
        patience: Duration::from_secs(60),
        ..Policy::quick()
    }
}

// Waiting, and the ends of a transfer.

/// The longest a test waits for something that has to happen.
const WAIT: Duration = Duration::from_secs(30);

/// Waits for `work`, but not for ever.
pub(super) async fn within<T>(what: &str, work: impl Future<Output = T>) -> T {
    timeout(WAIT, work)
        .await
        .unwrap_or_else(|_| panic!("{what} took too long"))
}

/// Waits until `ready` is true, but not for ever.
pub(super) async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    within(what, async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
}

/// Waits for the end of a transfer.
pub(super) async fn ended(sending: Sending) -> Outcome {
    within("the transfer to end", sending.outcome()).await
}

/// The id of the transfer `n`.
pub(super) fn id(n: u8) -> TransferId {
    TransferId::from_bytes([n; TransferId::LEN])
}
