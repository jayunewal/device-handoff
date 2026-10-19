// SPDX-License-Identifier: Apache-2.0

//! The sender between two real nodes on loopback.
//!
//! The tests of the engine (`receiver_tests` and `script_tests`) settle what it does when the other
//! device or the network does something. These show that a transfer from
//! [`Node::send_files`](crate::Node::send_files) gets to the other node's folder over real
//! connections, and that a connection which goes, a device that is unpaired, a person who gives the
//! transfer up and a node that stops end it as they should, with nothing left on either side.
//!
//! A transfer that is to be caught in the middle goes over a [`Held`] wire, which is the node's own
//! with the lanes held up once they have written some bytes, so that the test can do what it does at
//! a moment that it chooses and not at one that it hopes for.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use baton_proto::{DeviceId, close_code, id::TransferId, message::DeclineReason};
use tempfile::TempDir;

use super::{
    Error, NodeWire, Outcome, Policy, Sending, Stage, Why, Wire,
    testing::{Folder, Gate, Person, ended, held, steady, tree, until, within},
    wire::{LaneError, LaneOut, Link, Reply, WireError},
};
use crate::{
    Tuning,
    testing::{QUIET, TestNode},
    transfer::{
        destination::STAGING_PREFIX,
        incoming::{Ending, Verdict},
        testing::{bytes, destination},
    },
    util::BoxFuture,
};

/// A node that takes files, a paired node that sends them, and the folders of both.
struct Pair {
    sender: TestNode,
    receiver: TestNode,
    person: Arc<Person>,
    /// Where the receiver puts what it receives.
    received: PathBuf,
    /// The files to send.
    files: Folder,
    _received: TempDir,
}

impl Pair {
    async fn new() -> Self {
        let (folder, destination) = destination();
        let person = Arc::new(Person::new(destination));
        let receiver =
            TestNode::with(|builder| builder.transfer_handler(Arc::clone(&person))).await;
        let sender = TestNode::start().await;
        sender.pair_with(&receiver);
        Self {
            sender,
            receiver,
            person,
            received: folder.path().to_path_buf(),
            files: Folder::new(),
            _received: folder,
        }
    }

    /// Starts sending the items `names` of the folder, as the node does for whoever uses it.
    fn send(&self, names: &[&str]) -> Sending {
        self.sender
            .node
            .send_files_with(&self.receiver.id, self.files.walk(names), steady())
            .unwrap()
    }

    /// Starts sending the items `names` of the folder over a wire that holds the lanes up once they
    /// have written `after` bytes in all.
    fn send_held(&self, names: &[&str], after: u64) -> (Sending, Held) {
        let wire = Held::new(self.sender.node.wire_to(&self.receiver.id), after);
        let sending = self
            .sender
            .node
            .send_over(Arc::new(wire.clone()), self.files.walk(names), steady())
            .unwrap();
        (sending, wire)
    }

    /// Whether neither node has a connection to the other open.
    fn unconnected(&self) -> bool {
        self.sender.node.open_connections(&self.receiver.id) == 0
            && self.receiver.node.open_connections(&self.sender.id) == 0
    }
}

/// How many files the staging folders of `root` hold, in all the folders below them.
fn staged_files(root: &Path) -> usize {
    fn files_in(dir: &Path) -> usize {
        fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .map(|path| if path.is_dir() { files_in(&path) } else { 1 })
            .sum()
    }
    fs::read_dir(root)
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(STAGING_PREFIX)
        })
        .map(|holder| files_in(&holder.path()))
        .sum()
}

/// The node's own wire with the lanes held up: once they have written `after` bytes between them,
/// the next write waits until the test lets the lanes go. Only the lanes are held, and the requests
/// of a transfer are not.
#[derive(Clone)]
struct Held(Arc<HeldState>);

struct HeldState {
    inner: NodeWire,
    after: u64,
    written: AtomicU64,
    connections: AtomicUsize,
    /// Opened by the first write that has to wait.
    reached: Gate,
    /// Opened by the test.
    released: Gate,
}

impl Held {
    fn new(inner: NodeWire, after: u64) -> Self {
        Self(Arc::new(HeldState {
            inner,
            after,
            written: AtomicU64::new(0),
            connections: AtomicUsize::new(0),
            reached: Gate::new(),
            released: Gate::new(),
        }))
    }

    /// Waits until the lanes have written as much as they may.
    async fn reached(&self) {
        within("the lanes to be held up", self.0.reached.passed()).await;
    }

    /// Lets the lanes go on, now and for the rest of the transfer.
    fn release(&self) {
        self.0.released.open();
    }

    /// How many connections the transfer has made.
    fn connections(&self) -> usize {
        self.0.connections.load(Ordering::SeqCst)
    }
}

impl Wire for Held {
    fn connect(&self) -> BoxFuture<'_, Result<Box<dyn Link>, WireError>> {
        Box::pin(async move {
            let link = self.0.inner.connect().await?;
            self.0.connections.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(HeldLink {
                inner: link,
                held: Arc::clone(&self.0),
            }) as Box<dyn Link>)
        })
    }
}

struct HeldLink {
    inner: Box<dyn Link>,
    held: Arc<HeldState>,
}

impl Link for HeldLink {
    fn request(
        &self,
        request: baton_proto::transfer::TransferRequest,
        deadline: Duration,
    ) -> BoxFuture<'_, Result<Reply, WireError>> {
        self.inner.request(request, deadline)
    }

    fn open_lane(
        &self,
        id: TransferId,
        number: u8,
    ) -> BoxFuture<'_, Result<Box<dyn LaneOut>, WireError>> {
        Box::pin(async move {
            let lane = self.inner.open_lane(id, number).await?;
            Ok(Box::new(HeldLane {
                inner: lane,
                held: Arc::clone(&self.held),
            }) as Box<dyn LaneOut>)
        })
    }
}

struct HeldLane {
    inner: Box<dyn LaneOut>,
    held: Arc<HeldState>,
}

impl LaneOut for HeldLane {
    fn write<'a>(&'a mut self, bytes: &'a [u8]) -> BoxFuture<'a, Result<usize, LaneError>> {
        Box::pin(async move {
            if self.held.written.load(Ordering::SeqCst) >= self.held.after {
                self.held.reached.open();
                self.held.released.passed().await;
            }
            let taken = self.inner.write(bytes).await?;
            self.held
                .written
                .fetch_add(u64::try_from(taken).unwrap(), Ordering::SeqCst);
            Ok(taken)
        })
    }

    fn finish(&mut self) -> Result<(), LaneError> {
        self.inner.finish()
    }

    fn reset(&mut self, code: u32) {
        self.inner.reset(code);
    }
}

// What arrives.

#[tokio::test]
async fn folders_and_files_arrive_on_another_node_as_they_were() {
    let pair = Pair::new().await;
    let small = bytes(5, 40);
    let two_spans = bytes(3, 2 << 20);
    let a_byte_more = bytes(4, (1 << 20) + 1);
    pair.files.make("trip/notes.txt", &small);
    pair.files.make("trip/photos/one.jpg", &two_spans);
    pair.files.make("trip/photos/two.jpg", &a_byte_more);
    pair.files.make("trip/empty.txt", b"");
    pair.files.make_folder("trip/nothing");
    pair.files.make("top.bin", &small);
    let total = 40 + (2 << 20) + (1 << 20) + 1 + 40;
    let sending = pair.send(&["trip", "top.bin"]);
    let (id, progress) = (sending.id(), sending.progress());

    let outcome = ended(sending).await;

    let Outcome::Done(sent) = outcome else {
        panic!("{outcome:?}");
    };
    assert!(sent.done.failed.is_empty(), "{:?}", sent.done);
    assert_eq!((sent.files, sent.bytes), (5, total));
    assert_eq!(tree(&pair.received), tree(pair.files.root()));
    assert_eq!(pair.person.questions(), 1);
    let now = *progress.borrow();
    assert_eq!((now.stage, now.attempt, now.sent), (Stage::Over, 1, total));
    until("the receiver to hear the end", || {
        pair.person.endings().len() == 1
    })
    .await;
    assert_eq!(pair.person.endings(), [(id, Ending::Done(sent.done))]);
    // Nothing is left behind: no file in the staging folder, and no connection kept open for a
    // transfer that is over. The receiver keeps the empty folder of the transfer until it forgets it.
    assert_eq!(staged_files(&pair.received), 0);
    until("the connection to be let go of", || pair.unconnected()).await;
}

#[tokio::test]
async fn files_are_not_sent_to_a_device_that_is_not_paired() {
    let pair = Pair::new().await;
    let stranger = TestNode::start().await;
    pair.files.make("a.txt", b"hello");

    let refused = pair
        .sender
        .node
        .send_files_with(&stranger.id, pair.files.walk(&["a.txt"]), steady())
        .unwrap_err();

    assert!(matches!(refused, crate::Error::NotPaired(id) if id == stranger.id));
}

#[tokio::test]
async fn two_transfers_to_the_same_device_go_at_once_on_connections_of_their_own() {
    let pair = Pair::new().await;
    pair.files.make("one/a.bin", &bytes(8, 3 << 20));
    pair.files.make("two/b.bin", &bytes(9, 2 << 20));
    pair.files.make("two/c.txt", b"hello");
    let first = pair.send(&["one"]);
    let second = pair.send(&["two"]);
    assert_ne!(first.id(), second.id());

    let (first, second) = (ended(first).await, ended(second).await);

    for outcome in [first, second] {
        let Outcome::Done(sent) = outcome else {
            panic!("{outcome:?}");
        };
        assert!(sent.done.failed.is_empty(), "{:?}", sent.done);
    }
    assert_eq!(tree(&pair.received), tree(pair.files.root()));
    assert_eq!(pair.person.questions(), 2);
}

// What does not arrive, and why.

#[tokio::test]
async fn a_person_who_says_no_is_heard_as_a_decline_and_nothing_is_kept() {
    let pair = Pair::new().await;
    pair.files.make("a.txt", b"hello");
    pair.files.make("b.txt", b"hello again");
    for (name, verdict, reason) in [
        ("a.txt", Verdict::Refuse, DeclineReason::Refused),
        ("b.txt", Verdict::Block, DeclineReason::Blocked),
    ] {
        pair.person.says(verdict);

        let outcome = ended(pair.send(&[name])).await;

        assert_eq!(outcome, Outcome::Declined(reason));
    }
    assert_eq!(pair.person.questions(), 2);
    assert!(tree(&pair.received).is_empty());
    assert_eq!(held(&pair.received), 0);
    until("the connections to be let go of", || pair.unconnected()).await;
}

#[tokio::test]
async fn a_node_that_takes_no_files_declines_the_transfer_as_unsupported() {
    let receiver = TestNode::start().await;
    let sender = TestNode::start().await;
    sender.pair_with(&receiver);
    let files = Folder::new();
    files.make("a.txt", b"hello");

    let sending = sender
        .node
        .send_files_with(&receiver.id, files.walk(&["a.txt"]), steady())
        .unwrap();

    assert_eq!(
        ended(sending).await,
        Outcome::Declined(DeclineReason::Unsupported)
    );
}

#[tokio::test]
async fn a_device_that_is_not_paired_is_turned_away_before_anything_is_sent() {
    let sender = TestNode::start().await;
    let stranger = DeviceId::from_bytes([7; DeviceId::LEN]);
    let files = Folder::new();
    files.make("a.txt", b"hello");

    let error = sender
        .node
        .send_files(&stranger, files.walk(&["a.txt"]))
        .unwrap_err();

    assert!(
        matches!(error, crate::Error::NotPaired(id) if id == stranger),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_receiver_that_cannot_be_reached_is_given_up_on_when_patience_runs_out() {
    let receiver = TestNode::start().await;
    let sender = TestNode::with(|builder| {
        builder.tuning(Tuning {
            connect_timeout: Duration::from_millis(300),
            ..Tuning::default()
        })
    })
    .await;
    sender.pair_with(&receiver);
    receiver.node.shutdown().await;
    let files = Folder::new();
    files.make("a.txt", b"hello");
    let patience = Policy {
        patience: Duration::from_secs(1),
        ..steady()
    };

    let sending = sender
        .node
        .send_files_with(&receiver.id, files.walk(&["a.txt"]), patience)
        .unwrap();

    assert_eq!(
        ended(sending).await,
        Outcome::Failed(Error::GaveUp(Why::Connection))
    );
}

// A transfer in the middle.

#[tokio::test]
async fn a_connection_that_goes_in_the_middle_is_followed_by_a_new_one_that_finishes_the_transfer()
{
    let pair = Pair::new().await;
    pair.files.make("big.bin", &bytes(6, 4 << 20));
    pair.files.make("small.txt", b"hello");
    let (sending, wire) = pair.send_held(&["big.bin", "small.txt"], 1 << 20);
    let progress = sending.progress();
    wire.reached().await;

    // The network fails: the one connection that the transfer has is closed.
    assert_eq!(pair.sender.node.drop_connections(&pair.receiver.id), 1);
    wire.release();
    let outcome = ended(sending).await;

    let Outcome::Done(sent) = outcome else {
        panic!("{outcome:?}");
    };
    assert!(sent.done.failed.is_empty(), "{:?}", sent.done);
    assert_eq!(tree(&pair.received), tree(pair.files.root()));
    // The offer was made again on the new connection, and the receiver, which had decided on it,
    // did not ask the person again.
    assert_eq!(pair.person.questions(), 1);
    assert_eq!(wire.connections(), 2);
    assert_eq!(progress.borrow().attempt, 2);
    assert_eq!(staged_files(&pair.received), 0);
    until("the connections to be let go of", || pair.unconnected()).await;
}

#[tokio::test]
async fn a_device_that_is_unpaired_in_the_middle_ends_the_transfer_for_good() {
    let pair = Pair::new().await;
    pair.files.make("big.bin", &bytes(7, 4 << 20));
    let (sending, wire) = pair.send_held(&["big.bin"], 1 << 20);
    wire.reached().await;

    // The receiver forgets the sender, and closes its connections.
    pair.receiver.trust.remove(&pair.sender.id).unwrap();
    until("the receiver to close the connection", || {
        pair.sender.node.open_connections(&pair.receiver.id) == 0
    })
    .await;
    wire.release();
    let outcome = ended(sending).await;

    // The next connection is turned away, and a refusal is not tried again.
    assert_eq!(
        outcome,
        Outcome::Failed(Error::Refused(close_code::NOT_PAIRED))
    );
    until("the receiver to drop the transfer", || {
        pair.person.endings().len() == 1
    })
    .await;
    assert_eq!(pair.person.endings()[0].1, Ending::Dropped);
    until("the staging to go", || held(&pair.received) == 0).await;
    assert!(tree(&pair.received).is_empty());
    assert_eq!(wire.connections(), 1);
}

#[tokio::test]
async fn giving_a_transfer_up_tells_the_receiver_and_leaves_nothing_there() {
    let pair = Pair::new().await;
    pair.files.make("big.bin", &bytes(10, 4 << 20));
    let (sending, wire) = pair.send_held(&["big.bin"], 1 << 20);
    wire.reached().await;

    sending.cancel();
    let outcome = ended(sending).await;

    assert_eq!(outcome, Outcome::Cancelled { told: true });
    until("the receiver to drop the transfer", || {
        pair.person.endings().len() == 1
    })
    .await;
    assert_eq!(pair.person.endings()[0].1, Ending::Dropped);
    until("the staging to go", || held(&pair.received) == 0).await;
    assert!(tree(&pair.received).is_empty());
    until("the connection to be let go of", || pair.unconnected()).await;
}

#[tokio::test]
async fn letting_go_of_a_transfer_gives_it_up_like_cancelling_it() {
    let pair = Pair::new().await;
    pair.files.make("big.bin", &bytes(11, 4 << 20));
    let (sending, wire) = pair.send_held(&["big.bin"], 1 << 20);
    wire.reached().await;

    drop(sending);

    until("the receiver to drop the transfer", || {
        pair.person.endings().len() == 1
    })
    .await;
    assert_eq!(pair.person.endings()[0].1, Ending::Dropped);
    until("the staging to go", || held(&pair.received) == 0).await;
    until("the connection to be let go of", || pair.unconnected()).await;
}

#[tokio::test]
async fn stopping_the_node_gives_its_transfers_up_but_the_receiver_is_not_told() {
    let pair = Pair::new().await;
    pair.files.make("big.bin", &bytes(12, 4 << 20));
    let (sending, wire) = pair.send_held(&["big.bin"], 1 << 20);
    wire.reached().await;

    pair.sender.node.shutdown().await;
    let outcome = ended(sending).await;

    // The connections went with the node, so there was nothing to tell the receiver on. It keeps
    // what it holds, and does not end the transfer, until it forgets it.
    assert_eq!(outcome, Outcome::Cancelled { told: false });
    tokio::time::sleep(QUIET).await;
    assert_eq!(pair.person.endings(), Vec::new());
    assert!(held(&pair.received) > 0);
}
