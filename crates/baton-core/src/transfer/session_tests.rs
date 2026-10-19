// SPDX-License-Identifier: Apache-2.0

//! A node that takes files, reached over real connections on loopback.
//!
//! The other node speaks section 8 by hand: it asks what a sender asks, and opens lanes with the
//! bytes that a sender writes, in the order and with the endings that a test wants. That lets these
//! tests do what the sender of `send` never does, and show the receiving node alone. Whole
//! transfers between two nodes are in `send::node_tests`.

use std::{
    fs,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use baton_proto::{
    DeviceId, DeviceInfo, ItemId, Platform, TransferId, close_code,
    message::{
        DeclineReason, Handoff, HandoffReply, Item, Link, Outcome, Request, Response, capability,
    },
    transfer::{
        Accepted, Cancel, Cancelled, Declined, Offer, TransferReply, TransferRequest,
        lane::LaneHeader,
    },
};
use iroh::endpoint::{Connection, SendStream};

use super::{
    destination::Destination,
    incoming::{Ending, LANES_GRANTED, TransferHandler, Verdict},
    testing::{
        WAIT, chunk, destination, lane, list, sample_entries, sample_finish, sample_lane,
        sample_offer, sample_x, span, staging_folders, until, within,
    },
};
use crate::{
    HandoffHandler, PairedDevice, Tuning,
    gate::code,
    session::call,
    testing::{QUIET, TestNode},
    util::{BoxFuture, lock},
};

/// The id of the transfer `n`.
fn id(n: u8) -> TransferId {
    TransferId::from_bytes([n; TransferId::LEN])
}

fn accepted(id: TransferId) -> TransferReply {
    TransferReply::Accepted(Accepted {
        id,
        lanes: LANES_GRANTED,
    })
}

fn declined(id: TransferId, reason: DeclineReason) -> TransferReply {
    TransferReply::Declined(Declined { id, reason })
}

/// Puts what it receives in a folder of its own, says the same to every offer, and keeps how each
/// transfer ended.
struct Folder {
    destination: Mutex<Option<Destination>>,
    verdict: Verdict,
    endings: Mutex<Vec<(TransferId, Ending)>>,
}

impl Folder {
    fn endings(&self) -> Vec<(TransferId, Ending)> {
        lock(&self.endings).clone()
    }
}

impl TransferHandler for Folder {
    fn destination(&self) -> Option<Destination> {
        lock(&self.destination).clone()
    }

    fn offered<'a>(&'a self, _from: &'a PairedDevice, _offer: &'a Offer) -> BoxFuture<'a, Verdict> {
        Box::pin(async move { self.verdict })
    }

    fn ended(&self, _from: &PairedDevice, id: TransferId, ending: Ending) {
        lock(&self.endings).push((id, ending));
    }
}

/// Takes links, and nothing else, from other devices.
struct TakesLinks;

impl HandoffHandler for TakesLinks {
    fn capabilities(&self) -> &'static [&'static str] {
        &[capability::LINK]
    }

    fn handle<'a>(&'a self, _from: &'a PairedDevice, _handoff: Handoff) -> BoxFuture<'a, Outcome> {
        Box::pin(async { Outcome::Accepted })
    }
}

/// How the node that takes files is set up.
struct Spec {
    /// What the person says to an offer.
    verdict: Verdict,
    /// Whether there is a folder to receive into.
    folder: bool,
    tuning: Tuning,
    /// The limit on items from one device in a minute, if it is not the default.
    items_per_minute: Option<u32>,
    /// Whether the node takes links too.
    takes_links: bool,
}

impl Spec {
    fn new() -> Self {
        Self {
            verdict: Verdict::Accept,
            folder: true,
            tuning: Tuning::default(),
            items_per_minute: None,
            takes_links: false,
        }
    }
}

/// A node that takes files, and a paired node that sends them.
struct Rig {
    receiver: TestNode,
    sender: TestNode,
    folder: Arc<Folder>,
    scratch: tempfile::TempDir,
}

impl Rig {
    async fn new() -> Self {
        Self::with(Spec::new()).await
    }

    async fn with(spec: Spec) -> Self {
        let (scratch, destination) = destination();
        let folder = Arc::new(Folder {
            destination: Mutex::new(spec.folder.then_some(destination)),
            verdict: spec.verdict,
            endings: Mutex::default(),
        });
        let receiver = TestNode::configured(
            |config| {
                if let Some(limit) = spec.items_per_minute {
                    config.handoff.max_items_per_minute = limit;
                }
            },
            |builder| {
                let builder = builder
                    .tuning(spec.tuning)
                    .transfer_handler(Arc::clone(&folder));
                if spec.takes_links {
                    builder.handoff_handler(TakesLinks)
                } else {
                    builder
                }
            },
        )
        .await;
        let sender = TestNode::start().await;
        sender.pair_with(&receiver);
        Self {
            receiver,
            sender,
            folder,
            scratch,
        }
    }

    /// The connection that the sender keeps to the receiver, made if there is none.
    async fn connection(&self) -> Connection {
        self.sender
            .node
            .pooled_session(&self.receiver.id)
            .await
            .unwrap()
            .connection
    }

    /// Whether the receiver says in its hello that it takes files.
    async fn says_it_takes_files(&self) -> bool {
        let session = self
            .sender
            .node
            .pooled_session(&self.receiver.id)
            .await
            .unwrap();
        session
            .peer
            .capabilities
            .iter()
            .any(|name| name == capability::TRANSFER_FILES)
    }

    /// Whether anything of a transfer that is being received is on the disk.
    fn staging(&self) -> usize {
        staging_folders(self.scratch.path())
    }
}

/// Sends a request about a transfer and gives the answer.
async fn ask(connection: &Connection, request: TransferRequest) -> crate::Result<TransferReply> {
    match call(connection, &Request::Transfer(request), WAIT).await? {
        Response::Transfer(reply) => Ok(reply),
        other => panic!("the answer was not about a transfer: {other:?}"),
    }
}

async fn offer(connection: &Connection, transfer: TransferId) -> TransferReply {
    ask(connection, TransferRequest::Offer(sample_offer(transfer)))
        .await
        .unwrap()
}

async fn finish(connection: &Connection, transfer: TransferId) -> TransferReply {
    ask(connection, TransferRequest::Finish(sample_finish(transfer)))
        .await
        .unwrap()
}

/// Opens a lane of the transfer and writes its number and then `frames`. It is not ended.
async fn open_lane(
    connection: &Connection,
    transfer: TransferId,
    number: u8,
    frames: &[u8],
) -> SendStream {
    let mut send = within("opening a lane", connection.open_uni())
        .await
        .unwrap();
    let header = LaneHeader {
        transfer,
        lane: number,
    };
    send.write_all(&header.to_bytes()).await.unwrap();
    send.write_all(frames).await.unwrap();
    send
}

/// The code that the receiver stopped `send` with.
async fn stopped_with(send: &SendStream) -> u64 {
    within("the lane to be stopped", send.stopped())
        .await
        .unwrap()
        .expect("the lane was read to its end and not stopped")
        .into_inner()
}

/// Checks that the receiver let go of `send` the way it may once a transfer is done: it read the lane
/// to its end, or it stopped the lane with 499 because the transfer was over before it saw the end.
/// The end of a lane can reach the receiver after `Finish` has, and QUIC tells the sender of a stop
/// for as long as it has not had the end acknowledged. Another code would be a fault.
async fn assert_let_go(send: &SendStream) {
    let stopped = within("the lane to be let go", send.stopped())
        .await
        .unwrap();
    if let Some(code) = stopped {
        assert_eq!(
            code.into_inner(),
            u64::from(close_code::CANCELLED),
            "the lane was stopped with a fault"
        );
    }
}

/// The sample up to the middle of `x`: the list and a span, and no end to the file.
fn most_of_the_sample() -> Vec<u8> {
    lane(&[
        &list(&chunk(sample_entries())),
        &span(0, 0, &sample_x()[..4]),
    ])
}

// What a node says it takes.

#[tokio::test]
async fn only_a_node_with_a_folder_to_receive_into_says_it_takes_files() {
    assert!(Rig::new().await.says_it_takes_files().await);

    let no_folder = Rig::with(Spec {
        folder: false,
        ..Spec::new()
    })
    .await;
    assert!(!no_folder.says_it_takes_files().await);

    // A node that was given no handler at all says nothing of the kind.
    let (a, b) = (TestNode::start().await, TestNode::start().await);
    a.pair_with(&b);
    let session = a.node.pooled_session(&b.id).await.unwrap();
    assert!(
        !session
            .peer
            .capabilities
            .iter()
            .any(|name| name == capability::TRANSFER_FILES)
    );
}

#[tokio::test]
async fn a_node_with_no_folder_declines_every_request_about_a_transfer_as_unsupported() {
    let rig = Rig::with(Spec {
        folder: false,
        ..Spec::new()
    })
    .await;
    let connection = rig.connection().await;
    let transfer = id(4);

    for request in [
        TransferRequest::Offer(sample_offer(transfer)),
        TransferRequest::Finish(sample_finish(transfer)),
        TransferRequest::Cancel(Cancel { id: transfer }),
    ] {
        assert_eq!(
            ask(&connection, request).await.unwrap(),
            declined(transfer, DeclineReason::Unsupported)
        );
    }
}

// A transfer from the offer to the end.

#[tokio::test]
async fn a_transfer_goes_from_the_offer_to_done_and_its_files_are_in_place() {
    let rig = Rig::new().await;
    let connection = rig.connection().await;
    let transfer = id(1);

    assert_eq!(offer(&connection, transfer).await, accepted(transfer));
    let mut lane = open_lane(&connection, transfer, 0, &sample_lane()).await;
    lane.finish().unwrap();
    let TransferReply::Done(done) = finish(&connection, transfer).await else {
        panic!("the transfer did not end in Done");
    };

    assert!(done.failed.is_empty() && done.more_failed == 0, "{done:?}");
    assert_let_go(&lane).await;
    assert_eq!(fs::read(rig.scratch.path().join("x")).unwrap(), sample_x());
    assert_eq!(fs::read(rig.scratch.path().join("zero")).unwrap(), b"");
    // The application is told once, with the same outcome.
    until("the application to be told", || {
        !rig.folder.endings().is_empty()
    })
    .await;
    assert_eq!(rig.folder.endings(), [(transfer, Ending::Done(done))]);
}

#[tokio::test]
async fn a_transfer_that_was_taken_is_finished_even_if_the_folder_goes_away_after() {
    let rig = Rig::new().await;
    let connection = rig.connection().await;
    let transfer = id(1);
    assert_eq!(offer(&connection, transfer).await, accepted(transfer));

    *lock(&rig.folder.destination) = None;
    let mut lane = open_lane(&connection, transfer, 0, &sample_lane()).await;
    lane.finish().unwrap();

    assert!(matches!(
        finish(&connection, transfer).await,
        TransferReply::Done(done) if done.failed.is_empty()
    ));
    // A new offer is another matter: there is no folder now.
    assert_eq!(
        offer(&connection, id(2)).await,
        declined(id(2), DeclineReason::Unsupported)
    );
}

#[tokio::test]
async fn a_node_that_lost_its_folder_still_cancels_a_transfer_it_took() {
    let rig = Rig::new().await;
    let connection = rig.connection().await;
    let transfer = id(1);
    assert_eq!(offer(&connection, transfer).await, accepted(transfer));
    let lane = open_lane(&connection, transfer, 0, &most_of_the_sample()).await;
    until("the file to be started", || rig.staging() > 0).await;

    *lock(&rig.folder.destination) = None;
    let reply = ask(
        &connection,
        TransferRequest::Cancel(Cancel { id: transfer }),
    )
    .await;

    assert_eq!(
        reply.unwrap(),
        TransferReply::Cancelled(Cancelled { id: transfer })
    );
    assert_eq!(stopped_with(&lane).await, u64::from(close_code::CANCELLED));
    assert_eq!(rig.staging(), 0);
}

#[tokio::test]
async fn a_finish_for_a_transfer_that_is_not_there_is_answered_not_found() {
    let rig = Rig::new().await;
    let connection = rig.connection().await;

    let error = ask(&connection, TransferRequest::Finish(sample_finish(id(3))))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("NotFound"), "{error}");
}

#[tokio::test]
async fn a_finish_that_the_files_do_not_settle_for_is_stopped_with_408_and_asked_again() {
    let rig = Rig::with(Spec {
        tuning: Tuning {
            finish_wait: Duration::from_millis(300),
            ..Tuning::default()
        },
        ..Spec::new()
    })
    .await;
    let connection = rig.connection().await;
    let transfer = id(1);
    assert_eq!(offer(&connection, transfer).await, accepted(transfer));

    // No lane has come, so the files do not settle and the request ends without an answer.
    let error = ask(
        &connection,
        TransferRequest::Finish(sample_finish(transfer)),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("code 408"), "{error}");

    // The transfer is not over: the lanes come, and the sender asks again, as often as it takes.
    let mut lane = open_lane(&connection, transfer, 0, &sample_lane()).await;
    lane.finish().unwrap();
    let done = within("the transfer to be done", async {
        loop {
            match ask(
                &connection,
                TransferRequest::Finish(sample_finish(transfer)),
            )
            .await
            {
                Ok(TransferReply::Done(done)) => return done,
                Ok(other) => panic!("the answer was not Done: {other:?}"),
                Err(_) => {}
            }
        }
    })
    .await;
    assert!(done.failed.is_empty() && done.more_failed == 0, "{done:?}");
}

#[tokio::test]
async fn a_cancel_for_a_transfer_that_is_not_there_is_answered_cancelled() {
    let rig = Rig::new().await;
    let connection = rig.connection().await;

    let reply = ask(&connection, TransferRequest::Cancel(Cancel { id: id(3) })).await;

    assert_eq!(
        reply.unwrap(),
        TransferReply::Cancelled(Cancelled { id: id(3) })
    );
}

#[tokio::test]
async fn a_cancel_stops_the_lanes_and_removes_what_was_received() {
    let rig = Rig::new().await;
    let connection = rig.connection().await;
    let transfer = id(1);
    assert_eq!(offer(&connection, transfer).await, accepted(transfer));
    let lane = open_lane(&connection, transfer, 0, &most_of_the_sample()).await;
    until("the file to be started", || rig.staging() > 0).await;

    let reply = ask(
        &connection,
        TransferRequest::Cancel(Cancel { id: transfer }),
    )
    .await;

    assert_eq!(
        reply.unwrap(),
        TransferReply::Cancelled(Cancelled { id: transfer })
    );
    assert_eq!(stopped_with(&lane).await, u64::from(close_code::CANCELLED));
    assert_eq!(rig.staging(), 0);
    assert!(!rig.scratch.path().join("x").exists());
    until("the application to be told", || {
        !rig.folder.endings().is_empty()
    })
    .await;
    assert_eq!(rig.folder.endings(), [(transfer, Ending::Dropped)]);
}

// Lanes.

#[tokio::test]
async fn a_lane_for_a_transfer_that_is_not_taken_is_stopped_with_499() {
    let rig = Rig::with(Spec {
        verdict: Verdict::Refuse,
        ..Spec::new()
    })
    .await;
    let connection = rig.connection().await;

    // A transfer that was never offered, and one that the person turned down.
    let never = open_lane(&connection, id(7), 0, &sample_lane()).await;
    assert_eq!(
        offer(&connection, id(8)).await,
        declined(id(8), DeclineReason::Refused)
    );
    let refused = open_lane(&connection, id(8), 0, &sample_lane()).await;

    for send in [never, refused] {
        assert_eq!(stopped_with(&send).await, u64::from(close_code::CANCELLED));
    }
}

#[tokio::test]
async fn a_lane_that_never_says_its_number_is_stopped_with_408() {
    let rig = Rig::with(Spec {
        tuning: Tuning {
            request_deadline: Duration::from_millis(300),
            ..Tuning::default()
        },
        ..Spec::new()
    })
    .await;
    let connection = rig.connection().await;
    let transfer = id(1);
    assert_eq!(offer(&connection, transfer).await, accepted(transfer));

    // The 17 bytes that the stream starts with, and no number.
    let mut send = within("opening a lane", connection.open_uni())
        .await
        .unwrap();
    let header = LaneHeader { transfer, lane: 0 }.to_bytes();
    send.write_all(&header[..header.len() - 1]).await.unwrap();

    assert_eq!(stopped_with(&send).await, u64::from(close_code::TIMEOUT));
}

#[tokio::test]
async fn the_lanes_of_a_transfer_are_taken_on_the_connection_of_its_latest_offer() {
    let rig = Rig::new().await;
    let first = rig.connection().await;
    let transfer = id(1);
    assert_eq!(offer(&first, transfer).await, accepted(transfer));
    first.close(code(close_code::NORMAL), b"done with it");
    let second = rig.connection().await;

    // The attempt is on the first connection, so a lane on the second belongs to nothing yet.
    let too_early = open_lane(&second, transfer, 0, &sample_lane()).await;
    assert_eq!(
        stopped_with(&too_early).await,
        u64::from(close_code::CANCELLED)
    );

    // Offered again on the second, the transfer goes on there.
    assert_eq!(offer(&second, transfer).await, accepted(transfer));
    let mut lane = open_lane(&second, transfer, 0, &sample_lane()).await;
    lane.finish().unwrap();
    assert!(matches!(
        finish(&second, transfer).await,
        TransferReply::Done(done) if done.failed.is_empty()
    ));
}

#[tokio::test]
async fn lanes_that_are_idle_when_the_transfer_is_done_are_let_go_before_done_is_answered() {
    let rig = Rig::new().await;
    let connection = rig.connection().await;
    let transfer = id(1);
    assert_eq!(offer(&connection, transfer).await, accepted(transfer));
    let mut busy = open_lane(&connection, transfer, 0, &sample_lane()).await;
    busy.finish().unwrap();
    // The other lanes are open, and have nothing to say.
    let mut idle = Vec::new();
    for number in 1..LANES_GRANTED {
        idle.push(open_lane(&connection, transfer, number, &[]).await);
    }

    let asked = Instant::now();
    let reply = finish(&connection, transfer).await;

    // An answer waits for the lanes for a few seconds at most, and these were stopped at once.
    assert!(matches!(reply, TransferReply::Done(_)), "{reply:?}");
    assert!(
        asked.elapsed() < Duration::from_secs(3),
        "{:?}",
        asked.elapsed()
    );
    for send in &idle {
        assert_eq!(stopped_with(send).await, u64::from(close_code::CANCELLED));
    }
}

/// Offers the sample, sends it on all of the lanes that were granted, and finishes it.
async fn one_transfer(connection: &Connection, transfer: TransferId) {
    assert_eq!(offer(connection, transfer).await, accepted(transfer));
    let mut lanes = Vec::new();
    for number in 0..LANES_GRANTED {
        let frames = if number == 0 {
            sample_lane()
        } else {
            Vec::new()
        };
        let mut send = open_lane(connection, transfer, number, &frames).await;
        send.finish().unwrap();
        lanes.push(send);
    }
    let reply = finish(connection, transfer).await;
    assert!(
        matches!(&reply, TransferReply::Done(done) if done.failed.is_empty() && done.more_failed == 0),
        "{reply:?}"
    );
    for send in &lanes {
        assert_let_go(send).await;
    }
}

#[tokio::test]
async fn transfers_that_follow_each_other_find_the_lanes_of_the_last_let_go() {
    let rig = Rig::new().await;
    let connection = rig.connection().await;

    // A device may have two going at once, which are eight lanes. Each of these starts the next the
    // moment it has its `Done`.
    let run = |first: u8| {
        let connection = connection.clone();
        async move {
            for round in 0..5 {
                one_transfer(&connection, id(first + round)).await;
            }
        }
    };
    tokio::join!(run(10), run(20));

    until("the application to be told of all ten", || {
        rig.folder.endings().len() == 10
    })
    .await;
    assert!(
        rig.folder
            .endings()
            .iter()
            .all(|(_, ending)| matches!(ending, Ending::Done(_)))
    );
}

#[tokio::test]
async fn a_device_with_two_transfers_going_has_all_eight_lanes_open_and_a_ninth_stream_waits() {
    let rig = Rig::new().await;
    let connection = rig.connection().await;
    let transfers = [id(1), id(2)];
    for transfer in transfers {
        assert_eq!(offer(&connection, transfer).await, accepted(transfer));
    }

    // A sender that has both going has every lane of both open, and none of them has ended.
    let mut lanes = Vec::new();
    for transfer in transfers {
        for number in 0..LANES_GRANTED {
            let frames = if number == 0 {
                sample_lane()
            } else {
                Vec::new()
            };
            lanes.push(open_lane(&connection, transfer, number, &frames).await);
        }
    }

    // That is as many streams as the connection allows. A ninth waits for one of these to end.
    let ninth = tokio::time::timeout(QUIET, connection.open_uni()).await;
    assert!(ninth.is_err(), "a ninth stream was let through");

    for send in &mut lanes {
        send.finish().unwrap();
    }
    for transfer in transfers {
        let reply = finish(&connection, transfer).await;
        assert!(
            matches!(&reply, TransferReply::Done(done) if done.failed.is_empty() && done.more_failed == 0),
            "{reply:?}"
        );
    }
    for send in &lanes {
        assert_let_go(send).await;
    }
}

// The end of a node and of a pairing.

#[tokio::test]
async fn unpairing_a_device_gives_up_its_transfers_and_removes_what_was_received() {
    let rig = Rig::new().await;
    let connection = rig.connection().await;
    let transfer = id(1);
    assert_eq!(offer(&connection, transfer).await, accepted(transfer));
    let _lane = open_lane(&connection, transfer, 0, &most_of_the_sample()).await;
    until("the file to be started", || rig.staging() > 0).await;

    rig.receiver.trust.remove(&rig.sender.id).unwrap();

    until("the application to be told", || {
        !rig.folder.endings().is_empty()
    })
    .await;
    assert_eq!(rig.folder.endings(), [(transfer, Ending::Dropped)]);
    until("what was received to go", || rig.staging() == 0).await;
    within("the connection to be closed", connection.closed()).await;
}

#[tokio::test]
async fn a_node_that_missed_changes_to_its_pairings_gives_up_the_transfers_of_devices_that_are_gone()
 {
    let rig = Rig::new().await;
    let connection = rig.connection().await;
    let transfer = id(1);
    assert_eq!(offer(&connection, transfer).await, accepted(transfer));
    let _lane = open_lane(&connection, transfer, 0, &most_of_the_sample()).await;
    until("the file to be started", || rig.staging() > 0).await;

    unpair_and_lose_the_news(&rig, &rig.sender.id);

    until("the application to be told", || {
        !rig.folder.endings().is_empty()
    })
    .await;
    assert_eq!(rig.folder.endings(), [(transfer, Ending::Dropped)]);
    until("what was received to go", || rig.staging() == 0).await;
}

/// Unpairs `device` from the receiver, and so many changes follow that the task which watches the
/// list of paired devices has lost the one about `device` by the time it runs. The test does not
/// wait between the changes, so the task does not run in between.
fn unpair_and_lose_the_news(rig: &Rig, device: &DeviceId) {
    let trust = &rig.receiver.trust;
    trust.remove(device).unwrap();
    let info = DeviceInfo {
        name: "Another device".to_owned(),
        platform: Platform::Linux,
        app_version: "0.0.0-test".to_owned(),
    };
    for n in 1..=40 {
        let churn = DeviceId::from_bytes([n; DeviceId::LEN]);
        trust.pair(churn, &info, 1).unwrap();
        trust.remove(&churn).unwrap();
    }
}

#[tokio::test]
async fn a_node_that_missed_changes_to_its_pairings_keeps_the_transfers_of_devices_that_are_still_paired()
 {
    let rig = Rig::with(Spec {
        tuning: Tuning {
            finish_wait: Duration::from_millis(300),
            ..Tuning::default()
        },
        ..Spec::new()
    })
    .await;
    let gone = rig.connection().await;
    let other = TestNode::start().await;
    other.pair_with(&rig.receiver);
    let kept = other
        .node
        .pooled_session(&rig.receiver.id)
        .await
        .unwrap()
        .connection;
    assert_eq!(offer(&gone, id(1)).await, accepted(id(1)));
    assert_eq!(offer(&kept, id(2)).await, accepted(id(2)));

    unpair_and_lose_the_news(&rig, &rig.sender.id);

    until("the application to be told", || {
        !rig.folder.endings().is_empty()
    })
    .await;
    // The transfer of the device that is still paired is known. Its `Finish` waits for files
    // that never come, and is stopped with 408 where a transfer that was given up has no answer
    // but `NotFound`.
    let error = ask(&kept, TransferRequest::Finish(sample_finish(id(2))))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("code 408"), "{error}");
    assert_eq!(rig.folder.endings(), [(id(1), Ending::Dropped)]);
}

#[tokio::test]
async fn stopping_the_node_gives_up_its_transfers_and_removes_what_was_received() {
    let rig = Rig::new().await;
    let connection = rig.connection().await;
    let transfer = id(1);
    assert_eq!(offer(&connection, transfer).await, accepted(transfer));
    let _lane = open_lane(&connection, transfer, 0, &most_of_the_sample()).await;
    until("the file to be started", || rig.staging() > 0).await;

    rig.receiver.node.shutdown().await;

    assert_eq!(rig.folder.endings(), [(transfer, Ending::Dropped)]);
    assert_eq!(rig.staging(), 0);
}

// What a transfer counts for.

fn a_link(n: u8) -> Handoff {
    Handoff {
        id: ItemId::from_bytes([n; ItemId::LEN]),
        item: Item::Link(Link {
            url: "https://example.com".to_owned(),
            title: None,
        }),
    }
}

/// What the receiver says to a link.
async fn hand_over(connection: &Connection, n: u8) -> Outcome {
    match call(connection, &Request::Handoff(a_link(n)), WAIT)
        .await
        .unwrap()
    {
        Response::Handoff(HandoffReply { outcome, .. }) => outcome,
        other => panic!("the answer was not about an item: {other:?}"),
    }
}

#[tokio::test]
async fn an_offer_counts_as_an_item_against_the_limit_on_items_and_so_does_an_item_for_an_offer() {
    let spec = || Spec {
        items_per_minute: Some(2),
        takes_links: true,
        ..Spec::new()
    };

    // One item and one offer use up the limit, so the next item is turned away.
    let rig = Rig::with(spec()).await;
    let connection = rig.connection().await;
    assert_eq!(hand_over(&connection, 1).await, Outcome::Accepted);
    assert_eq!(offer(&connection, id(1)).await, accepted(id(1)));
    assert_eq!(
        hand_over(&connection, 2).await,
        Outcome::Declined(DeclineReason::RateLimited)
    );

    // Two items use it up too, so the next offer is turned away, and is not remembered.
    let rig = Rig::with(spec()).await;
    let connection = rig.connection().await;
    assert_eq!(hand_over(&connection, 1).await, Outcome::Accepted);
    assert_eq!(hand_over(&connection, 2).await, Outcome::Accepted);
    assert_eq!(
        offer(&connection, id(1)).await,
        declined(id(1), DeclineReason::RateLimited)
    );
    assert_eq!(rig.folder.endings(), Vec::new());
}
