// SPDX-License-Identifier: Apache-2.0

//! The wire over a node's connections, between two real nodes on loopback: what a request and a lane
//! come to when the other device answers, takes its time, stops the lane, ends the connection, or
//! does not know this device. What the engine makes of each is tested with a scripted wire
//! (`script_tests`); these show that the real wire says it.

use std::{sync::Arc, time::Duration};

use baton_proto::{
    close_code,
    message::{DeclineReason, ErrorCode},
    transfer::{Cancel, Cancelled, Declined, TransferReply, TransferRequest},
};
use tempfile::TempDir;

use super::{LaneError, LaneOut, Link, NodeWire, QuicLink, Reply, WireError};
use crate::{
    Tuning,
    testing::{QUIET, TestNode},
    transfer::{
        incoming::Ending,
        send::testing::{Person, id, until, within},
        testing::{WAIT, destination, sample_finish, sample_offer},
    },
};

/// A node that takes files, a paired node that sends them, and the person at the receiving end.
struct Nodes {
    sender: TestNode,
    receiver: TestNode,
    person: Arc<Person>,
    _received: TempDir,
}

impl Nodes {
    async fn new() -> Self {
        Self::with(Tuning::default()).await
    }

    /// The receiver works to `tuning`.
    async fn with(tuning: Tuning) -> Self {
        let (folder, destination) = destination();
        let person = Arc::new(Person::new(destination));
        let receiver =
            TestNode::with(|builder| builder.tuning(tuning).transfer_handler(Arc::clone(&person)))
                .await;
        let sender = TestNode::start().await;
        sender.pair_with(&receiver);
        Self {
            sender,
            receiver,
            person,
            _received: folder,
        }
    }

    fn wire(&self) -> NodeWire {
        self.sender.node.wire_to(&self.receiver.id)
    }

    /// A connection to the receiver, as the engine makes one.
    async fn link(&self) -> QuicLink {
        match within("connecting", self.wire().link()).await {
            Ok(link) => link,
            Err(error) => panic!("the connection could not be made: {error}"),
        }
    }

    /// Whether neither node has a connection to the other open.
    fn unconnected(&self) -> bool {
        self.sender.node.open_connections(&self.receiver.id) == 0
            && self.receiver.node.open_connections(&self.sender.id) == 0
    }
}

// Requests.

#[tokio::test]
async fn a_request_is_answered_by_the_receiver_on_a_stream_of_its_own() {
    let nodes = Nodes::new().await;
    let link = nodes.link().await;

    let offered = link
        .request(TransferRequest::Offer(sample_offer(id(1))), WAIT)
        .await;
    let Ok(Reply::Transfer(TransferReply::Accepted(accepted))) = offered else {
        panic!("{offered:?}");
    };
    assert_eq!(accepted.id, id(1));
    assert_eq!(nodes.person.questions(), 1);

    // An answer that is an error is handed on with its code, on the same connection.
    let asked = link
        .request(TransferRequest::Finish(sample_finish(id(2))), WAIT)
        .await;
    assert_eq!(asked, Ok(Reply::Error(ErrorCode::NotFound)));
}

#[tokio::test]
async fn a_request_that_gets_no_answer_in_time_is_dropped_and_can_be_made_again() {
    let nodes = Nodes::new().await;
    let gate = nodes.person.takes_their_time();
    let link = nodes.link().await;

    let asked = link
        .request(
            TransferRequest::Offer(sample_offer(id(1))),
            Duration::from_millis(200),
        )
        .await;

    assert_eq!(asked, Err(WireError::TimedOut));
    // The stream that the answer was to come on is let go of, and the question stays with the
    // person: the same offer made again finds it under way, and waits for the same answer.
    let (again, ()) = tokio::join!(
        link.request(TransferRequest::Offer(sample_offer(id(1))), WAIT),
        async {
            tokio::time::sleep(QUIET).await;
            assert_eq!(nodes.person.questions(), 1);
            assert_eq!(nodes.person.withdrawn(), 0);
            gate.open();
        }
    );
    let Ok(Reply::Transfer(TransferReply::Accepted(accepted))) = again else {
        panic!("{again:?}");
    };
    assert_eq!(accepted.id, id(1));
    assert_eq!(nodes.person.questions(), 1);
}

#[tokio::test]
async fn a_request_that_the_receiver_gives_up_waiting_on_is_one_to_make_again() {
    // The files of the transfer never come, so the `Finish` is not answered, and the receiver ends
    // the request with a stop of 408 (section 8, Finish).
    let nodes = Nodes::with(Tuning {
        finish_wait: Duration::from_millis(150),
        ..Tuning::default()
    })
    .await;
    let link = nodes.link().await;
    let offered = link
        .request(TransferRequest::Offer(sample_offer(id(1))), WAIT)
        .await;
    assert!(matches!(
        offered,
        Ok(Reply::Transfer(TransferReply::Accepted(_)))
    ));

    let finished = link
        .request(TransferRequest::Finish(sample_finish(id(1))), WAIT)
        .await;

    assert_eq!(finished, Err(WireError::TimedOut));
}

#[tokio::test]
async fn a_device_that_takes_no_files_now_is_offered_none_but_is_told_to_let_go_of_what_it_holds() {
    let nodes = Nodes::new().await;
    // A transfer that the device took while it took files, and that it still holds.
    let first = nodes.link().await;
    let offered = first
        .request(TransferRequest::Offer(sample_offer(id(1))), WAIT)
        .await;
    assert!(matches!(
        offered,
        Ok(Reply::Transfer(TransferReply::Accepted(_)))
    ));
    // The person turns receiving off. The hello of a new connection says that it takes no files.
    nodes.person.stops_taking_files();
    let link = nodes.link().await;

    // An offer is not made, and the answer is the one that the device would give.
    let offered = link
        .request(TransferRequest::Offer(sample_offer(id(2))), WAIT)
        .await;
    assert_eq!(
        offered,
        Ok(Reply::Transfer(TransferReply::Declined(Declined {
            id: id(2),
            reason: DeclineReason::Unsupported,
        })))
    );
    assert_eq!(nodes.person.questions(), 1);

    // `Finish` of what it holds goes to the device too, which waits for files that never come.
    let finished = link
        .request(
            TransferRequest::Finish(sample_finish(id(1))),
            Duration::from_secs(1),
        )
        .await;
    assert_eq!(finished, Err(WireError::TimedOut));
    // So does `Cancel`. The device gives up what it holds, and says so.
    let cancelled = link
        .request(TransferRequest::Cancel(Cancel { id: id(1) }), WAIT)
        .await;
    assert_eq!(
        cancelled,
        Ok(Reply::Transfer(TransferReply::Cancelled(Cancelled {
            id: id(1)
        })))
    );
    until("the person to hear that the transfer is dropped", || {
        nodes.person.endings() == [(id(1), Ending::Dropped)]
    })
    .await;
    // What it does not hold is declined as unsupported, as it is for a device that takes no part
    // in transfers: there is nothing of it to give up.
    let unsupported = Ok(Reply::Transfer(TransferReply::Declined(Declined {
        id: id(3),
        reason: DeclineReason::Unsupported,
    })));
    let cancel = link
        .request(TransferRequest::Cancel(Cancel { id: id(3) }), WAIT)
        .await;
    assert_eq!(cancel, unsupported);
    let finish = link
        .request(TransferRequest::Finish(sample_finish(id(3))), WAIT)
        .await;
    assert_eq!(finish, unsupported);
}

// Lanes.

#[tokio::test]
async fn a_lane_of_a_transfer_the_receiver_does_not_know_is_stopped_with_499() {
    let nodes = Nodes::new().await;
    let link = nodes.link().await;
    let mut lane = within("opening the lane", link.open(id(1), 0))
        .await
        .unwrap();

    // The writes go on until the stop has come back, and from then on they say so.
    let stopped = within("the lane to be stopped", async {
        loop {
            if let Err(error) = lane.write(&[7; 4096]).await {
                break error;
            }
        }
    })
    .await;

    assert_eq!(stopped, LaneError::Stopped(close_code::CANCELLED));
    assert_eq!(
        lane.write(b"more").await,
        Err(LaneError::Stopped(close_code::CANCELLED))
    );
    assert_eq!(
        lane.finish(),
        Err(LaneError::Stopped(close_code::CANCELLED))
    );
}

#[tokio::test]
async fn a_lane_that_was_stopped_when_nothing_was_written_says_so_when_it_is_ended() {
    let nodes = Nodes::new().await;
    let link = nodes.link().await;
    let mut lane = within("opening the lane", link.open(id(1), 0))
        .await
        .unwrap();

    // Ending a stream that was stopped is not an error to QUIC. The lane looks for the stop first.
    until("the stop to come back", || lane.stop_by_now().is_some()).await;

    assert_eq!(lane.stop_by_now(), Some(close_code::CANCELLED));
    assert_eq!(
        lane.finish(),
        Err(LaneError::Stopped(close_code::CANCELLED))
    );
}

#[tokio::test]
async fn a_lane_is_sent_after_the_requests_that_are_waiting() {
    let nodes = Nodes::new().await;
    let link = nodes.link().await;

    let lane = within("opening the lane", link.open(id(1), 0))
        .await
        .unwrap();

    // A request goes on a stream of the priority 0, which is a new stream's.
    assert!(lane.send.priority().unwrap() < 0);
}

#[tokio::test]
async fn a_lane_that_is_let_go_of_before_it_was_ended_is_reset_and_not_finished() {
    // A request that waits for files the receiver stops waiting for after this long.
    let nodes = Nodes::with(Tuning {
        finish_wait: Duration::from_millis(150),
        ..Tuning::default()
    })
    .await;
    let link = nodes.link().await;
    let offered = link
        .request(TransferRequest::Offer(sample_offer(id(1))), WAIT)
        .await;
    assert!(matches!(
        offered,
        Ok(Reply::Transfer(TransferReply::Accepted(_)))
    ));
    let lane = within("opening the lane", link.open(id(1), 0))
        .await
        .unwrap();

    // QUIC finishes a stream that is dropped, and the receiver would take lane 0 for one that ended
    // before its list was whole, which is a violation that gives the transfer up.
    drop(lane);

    // A transfer that is still there waits for its files. One that was given up is not found.
    let finished = link
        .request(TransferRequest::Finish(sample_finish(id(1))), WAIT)
        .await;
    assert_eq!(finished, Err(WireError::TimedOut));
}

// Connections.

#[tokio::test]
async fn a_connection_is_closed_when_the_link_is_let_go_of() {
    let nodes = Nodes::new().await;
    let link = nodes.link().await;
    assert_eq!(nodes.sender.node.open_connections(&nodes.receiver.id), 1);
    until("the receiver to see the connection", || {
        nodes.receiver.node.open_connections(&nodes.sender.id) == 1
    })
    .await;

    drop(link);

    until("both ends to see it closed", || nodes.unconnected()).await;
}

#[tokio::test]
async fn a_connection_is_closed_when_the_link_is_let_go_of_by_a_node_that_takes_files_too() {
    // An app that sends files takes them too. Its node then reads the unidirectional streams of
    // every connection it makes, with a handle of its own to each, so letting go of the link's
    // handle does not end the connection by itself.
    let nodes = Nodes::new().await;
    let receiver = &nodes.receiver;
    let (_folder, destination) = destination();
    let sender = TestNode::with(|builder| builder.transfer_handler(Person::new(destination))).await;
    sender.pair_with(receiver);
    let link = match within("connecting", sender.node.wire_to(&receiver.id).link()).await {
        Ok(link) => link,
        Err(error) => panic!("the connection could not be made: {error}"),
    };
    until("the receiver to see the connection", || {
        receiver.node.open_connections(&sender.id) == 1
    })
    .await;

    drop(link);

    until("both ends to see it closed", || {
        sender.node.open_connections(&receiver.id) == 0
            && receiver.node.open_connections(&sender.id) == 0
    })
    .await;
}

#[tokio::test]
async fn a_device_that_does_not_know_this_one_is_refused_for_good() {
    let receiver = TestNode::start().await;
    let sender = TestNode::start().await;
    // The sender knows the receiver, and the receiver does not know the sender.
    sender.know(&receiver);
    let wire = sender.node.wire_to(&receiver.id);

    // The other device cannot know this one before it has the last message of the handshake, so the
    // refusal reaches the dialer only after the connection is its own: it comes with the answer to
    // the hello that opens every connection.
    let refused = within("connecting", wire.link()).await.err();

    assert_eq!(refused, Some(WireError::Refused(close_code::NOT_PAIRED)));
}

#[tokio::test]
async fn a_device_that_was_unpaired_since_is_not_dialed() {
    let nodes = Nodes::new().await;
    nodes.sender.trust.remove(&nodes.receiver.id).unwrap();

    let refused = within("connecting", nodes.wire().link()).await.err();

    assert_eq!(refused, Some(WireError::Refused(close_code::NOT_PAIRED)));
    assert_eq!(nodes.receiver.node.open_connections(&nodes.sender.id), 0);
}

#[tokio::test]
async fn a_device_that_is_gone_cannot_be_connected_to_and_is_not_refused() {
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

    let failed = within("connecting", sender.node.wire_to(&receiver.id).link())
        .await
        .err();

    assert!(
        matches!(failed, Some(WireError::TimedOut | WireError::Lost)),
        "{failed:?}"
    );
}
