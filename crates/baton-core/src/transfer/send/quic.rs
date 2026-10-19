// SPDX-License-Identifier: Apache-2.0

//! The wire over a node's connections.
//!
//! A [`NodeWire`] makes a session connection to one paired device each time the engine asks for one,
//! and a [`QuicLink`] is that connection. Requests go on streams of their own, one per request, and
//! a lane is a unidirectional stream that begins with the header of section 8 (Lanes).
//!
//! The connection is the transfer's own and not the one the node keeps for requests. A transfer that
//! is tried again goes on a new connection (section 8, Resuming), and one that has a connection of
//! its own cannot be cut off by the node closing an idle one, or by an ordinary request that finds
//! the pooled one dead and replaces it. The connection is closed when the link is let go of.
//!
//! What this module adds to the streams is the reading of what goes wrong with them, which is what
//! the engine decides by: a request that gets no answer in time is asked again on a new stream, a
//! connection that goes is replaced, and a connection that the other device ended because it does not
//! know this device, or because this device broke a rule, is not tried again.
//!
//! Lanes are given a lower priority than requests. Without it a `Finish` or a `Cancel` would wait
//! behind the file data that the connection has buffered, which on a slow link is seconds.

use std::{
    pin::pin,
    sync::Arc,
    task::{Context, Poll, Waker},
    time::Duration,
};

use baton_proto::{
    DeviceId, close_code,
    id::TransferId,
    message::{ErrorCode, Request, Response},
    transfer::{TransferRequest, lane::LaneHeader},
};
use iroh::{
    Endpoint,
    endpoint::{Connection, ConnectionError, SendStream, VarInt, WriteError},
};

use super::wire::{LaneError, LaneOut, Link, Reply, Wire, WireError};
use crate::{
    error::Error,
    gate::code,
    session,
    shared::Shared,
    util::BoxFuture,
    wire::{ReadFailure, code_of, connection_error, read_message, write_message},
};

/// The priority of a lane. A request has the priority of a new stream, which is 0, and goes ahead of
/// the lanes that are waiting to be sent.
const LANE_PRIORITY: i32 = -1;

/// The connections of a node to one paired device, as the engine asks for them.
pub(crate) struct NodeWire {
    endpoint: Endpoint,
    shared: Arc<Shared>,
    peer: DeviceId,
}

impl NodeWire {
    pub(crate) fn new(endpoint: Endpoint, shared: Arc<Shared>, peer: DeviceId) -> Self {
        Self {
            endpoint,
            shared,
            peer,
        }
    }

    /// A new connection to the device.
    async fn link(&self) -> Result<QuicLink, WireError> {
        // Looked up for each connection: a device that was unpaired since the last one is not
        // dialed, and one that was reached somewhere new since is dialed there.
        let Some(paired) = self.shared.trust.get(&self.peer) else {
            return Err(WireError::Refused(close_code::NOT_PAIRED));
        };
        let session = session::connect(&self.endpoint, &self.shared, &paired)
            .await
            .map_err(|error| connect_failure(&error))?;
        Ok(QuicLink {
            connection: session.connection,
        })
    }
}

impl Wire for NodeWire {
    fn connect(&self) -> BoxFuture<'_, Result<Box<dyn Link>, WireError>> {
        Box::pin(async move { Ok(Box::new(self.link().await?) as Box<dyn Link>) })
    }
}

/// A session connection that belongs to one transfer.
struct QuicLink {
    connection: Connection,
}

impl Drop for QuicLink {
    fn drop(&mut self) {
        // The attempt is over, and what its lanes still had to say is no longer wanted.
        self.connection.close(code(close_code::NORMAL), b"done");
    }
}

impl Link for QuicLink {
    fn request(
        &self,
        request: TransferRequest,
        deadline: Duration,
    ) -> BoxFuture<'_, Result<Reply, WireError>> {
        Box::pin(async move {
            let request = Request::Transfer(request);
            let asked = tokio::time::timeout(deadline, ask(&self.connection, &request, deadline));
            let response = asked.await.map_err(|_| WireError::TimedOut)??;
            Ok(reply_of(response))
        })
    }

    fn open_lane(
        &self,
        id: TransferId,
        number: u8,
    ) -> BoxFuture<'_, Result<Box<dyn LaneOut>, WireError>> {
        Box::pin(async move { Ok(Box::new(self.open(id, number).await?) as Box<dyn LaneOut>) })
    }
}

impl QuicLink {
    /// Opens lane `number` of the transfer `id` and writes its header.
    async fn open(&self, id: TransferId, number: u8) -> Result<QuicLane, WireError> {
        let send = self
            .connection
            .open_uni()
            .await
            .map_err(|error| connection_failure(&error))?;
        // A stream that is gone already cannot be given a priority, and the first write says what
        // became of it.
        let _ = send.set_priority(LANE_PRIORITY);
        // From here the lane is reset if the future is dropped before it returns it.
        let mut lane = QuicLane {
            send,
            stopped: None,
            over: false,
        };
        let header = LaneHeader {
            transfer: id,
            lane: number,
        };
        match lane.send.write_all(&header.to_bytes()).await {
            Ok(()) => {}
            // The receiver stopped the lane before its first byte, which the first write says.
            Err(WriteError::Stopped(stop)) => lane.stopped = Some(code_of(stop)),
            Err(error) => return Err(write_failure(&error)),
        }
        Ok(lane)
    }
}

/// A lane that is open for writing: a unidirectional stream whose header is written.
struct QuicLane {
    send: SendStream,
    /// The code that the receiver stopped the lane with, once it is known.
    stopped: Option<u32>,
    /// Whether the lane was ended or reset, so that letting go of it has nothing left to do.
    over: bool,
}

impl Drop for QuicLane {
    fn drop(&mut self) {
        // QUIC finishes a stream that is dropped, and the receiver would take a lane that was let go
        // of before its end for one that ended whole.
        if !self.over {
            let _ = self.send.reset(VarInt::from_u32(close_code::CANCELLED));
        }
    }
}

impl QuicLane {
    /// The code that the receiver has stopped the lane with by now, if it has. The stream is not
    /// waited for.
    fn stop_by_now(&self) -> Option<u32> {
        let stopped = self.send.stopped();
        let mut stopped = pin!(stopped);
        match stopped
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(Ok(Some(stop))) => Some(code_of(stop)),
            _ => None,
        }
    }
}

impl LaneOut for QuicLane {
    fn write<'a>(&'a mut self, bytes: &'a [u8]) -> BoxFuture<'a, Result<usize, LaneError>> {
        Box::pin(async move {
            if let Some(stop) = self.stopped {
                return Err(LaneError::Stopped(stop));
            }
            match self.send.write(bytes).await {
                Ok(taken) => Ok(taken),
                Err(WriteError::Stopped(stop)) => {
                    let stop = code_of(stop);
                    self.stopped = Some(stop);
                    Err(LaneError::Stopped(stop))
                }
                Err(_) => Err(LaneError::Lost),
            }
        })
    }

    fn finish(&mut self) -> Result<(), LaneError> {
        // Ending a stream that the receiver has stopped is not an error to QUIC and says nothing of
        // the stop, so the stop is looked for first.
        if let Some(stop) = self.stopped.or_else(|| self.stop_by_now()) {
            return Err(LaneError::Stopped(stop));
        }
        self.send.finish().map_err(|_| LaneError::Lost)?;
        self.over = true;
        Ok(())
    }

    fn reset(&mut self, stop: u32) {
        // A lane that is ended or reset already has nothing left to give up.
        let _ = self.send.reset(VarInt::from_u32(stop));
        self.over = true;
    }
}

/// Makes `request` on a stream of its own and reads the one response, waiting `deadline` at most
/// for it.
async fn ask(
    connection: &Connection,
    request: &Request,
    deadline: Duration,
) -> Result<Response, WireError> {
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|error| connection_failure(&error))?;
    write_message(&mut send, request)
        .await
        .map_err(|error| error_failure(&error))?;
    send.finish().map_err(|_| WireError::Lost)?;
    read_message::<Response>(&mut recv, deadline)
        .await
        .map_err(|failure| read_failure(&failure))
}

/// What the engine is told of the response to a request.
fn reply_of(response: Response) -> Reply {
    match response {
        Response::Transfer(reply) => Reply::Transfer(reply),
        Response::Error(error) => Reply::Error(error.code),
        // An answer of another kind is as far from the protocol as one that says it was not
        // understood.
        _ => Reply::Error(ErrorCode::Malformed),
    }
}

// What goes wrong, and what it means to the engine.

/// A connection that cannot be made.
fn connect_failure(error: &Error) -> WireError {
    match error {
        Error::Timeout(_) => WireError::TimedOut,
        other => error_failure(other),
    }
}

/// An error that is not about what a stream was told.
fn error_failure(error: &Error) -> WireError {
    match error {
        Error::Closed { code, .. } => closed(*code),
        _ => WireError::Lost,
    }
}

/// A stream could not be opened, or the connection went while it was in use.
fn connection_failure(error: &ConnectionError) -> WireError {
    error_failure(&connection_error("using the connection", error))
}

/// A write to a stream failed.
fn write_failure(error: &WriteError) -> WireError {
    match error {
        WriteError::ConnectionLost(error) => connection_failure(error),
        _ => WireError::Lost,
    }
}

/// A response that did not come, or not whole.
fn read_failure(failure: &ReadFailure) -> WireError {
    match failure {
        // No answer came in time, or the receiver gave up waiting for the files to settle (section 8,
        // Finish), and the request is to be made again.
        ReadFailure::TimedOut | ReadFailure::Reset(close_code::TIMEOUT) => WireError::TimedOut,
        // A response that cannot be read, or a request that the other device took for a violation,
        // will be the same on another connection.
        ReadFailure::Reset(close_code::PROTOCOL_VIOLATION)
        | ReadFailure::BadHeader(_)
        | ReadFailure::BadPayload(_) => WireError::Refused(close_code::PROTOCOL_VIOLATION),
        ReadFailure::Reset(_) => WireError::Lost,
        ReadFailure::Transport(error) => error_failure(error),
    }
}

/// The other device ended the connection with the code `by`.
fn closed(by: u64) -> WireError {
    match u32::try_from(by) {
        Ok(refusal @ (close_code::NOT_PAIRED | close_code::PROTOCOL_VIOLATION)) => {
            WireError::Refused(refusal)
        }
        _ => WireError::Lost,
    }
}

#[cfg(test)]
mod tests {
    use baton_proto::{
        frame::FrameError,
        message::{ErrorReply, Ping},
        transfer::{Cancelled, TransferReply},
    };
    use iroh::endpoint::{ReadError, ReadToEndError};

    use super::*;
    use crate::{
        testing::{Device, connect, next},
        uni::Kind,
    };

    fn closed_with(by: u32) -> Error {
        Error::Closed {
            code: u64::from(by),
            reason: "for a reason".into(),
        }
    }

    #[tokio::test]
    async fn a_lane_that_is_reset_reaches_the_other_device_reset_with_the_code_it_was_given() {
        let (receiver, mut taken) = Device::taking(&[(Kind::Lane, 2)]).await;
        let (sender, _) = Device::taking(&[]).await;
        let (dialed, _) = connect(&sender, &receiver).await;
        let link = QuicLink { connection: dialed };
        let mut lane = link.open(TransferId::from_bytes([5; 16]), 0).await.unwrap();
        // The other device has the lane by its start before the reset, which would take the start
        // with it if it came first.
        let mut arrival = next(&mut taken[0]).await;
        lane.write(b"the start of a frame").await.unwrap();

        lane.reset(close_code::TIMEOUT);

        // Not an end that looks whole: QUIC would finish a stream that was let go of.
        match arrival.recv.read_to_end(1024).await {
            Err(ReadToEndError::Read(ReadError::Reset(code))) => {
                assert_eq!(code.into_inner(), u64::from(close_code::TIMEOUT));
            }
            other => panic!("the lane did not arrive reset: {other:?}"),
        }
        // A lane that was reset takes nothing more.
        assert_eq!(lane.write(b"more").await, Err(LaneError::Lost));
    }

    #[test]
    fn a_response_is_handed_on_as_the_reply_it_is() {
        let cancelled = TransferReply::Cancelled(Cancelled {
            id: TransferId::from_bytes([3; 16]),
        });
        assert_eq!(
            reply_of(Response::Transfer(cancelled.clone())),
            Reply::Transfer(cancelled)
        );
        for code in [
            ErrorCode::NotFound,
            ErrorCode::Internal,
            ErrorCode::Malformed,
        ] {
            let error = ErrorReply {
                code,
                message: "for a reason".into(),
            };
            assert_eq!(reply_of(Response::Error(error)), Reply::Error(code));
        }
        // A response to some other request is none that a transfer can use.
        assert_eq!(
            reply_of(Response::Pong(Ping { nonce: 7 })),
            Reply::Error(ErrorCode::Malformed)
        );
    }

    #[test]
    fn a_connection_that_the_other_device_ended_for_good_is_not_tried_again() {
        for refusal in [close_code::NOT_PAIRED, close_code::PROTOCOL_VIOLATION] {
            let error = closed_with(refusal);
            assert_eq!(connect_failure(&error), WireError::Refused(refusal));
            assert_eq!(error_failure(&error), WireError::Refused(refusal));
            assert_eq!(
                read_failure(&ReadFailure::Transport(error)),
                WireError::Refused(refusal)
            );
        }
    }

    #[test]
    fn a_connection_that_went_for_any_other_reason_is_tried_again() {
        for other in [
            close_code::NORMAL,
            close_code::TIMEOUT,
            close_code::OVERLOADED,
            close_code::INTERNAL,
            close_code::CANCELLED,
        ] {
            assert_eq!(connect_failure(&closed_with(other)), WireError::Lost);
        }
        let too_big = Error::Closed {
            code: u64::from(u32::MAX) + 1,
            reason: String::new(),
        };
        assert_eq!(error_failure(&too_big), WireError::Lost);
        let network = Error::network("connecting", "no route");
        assert_eq!(connect_failure(&network), WireError::Lost);
        // A device that answers oddly is tried again as well, for it may be shutting down.
        let odd = Error::protocol("the answer to hello was not a hello");
        assert_eq!(connect_failure(&odd), WireError::Lost);
    }

    #[test]
    fn a_connection_that_could_not_be_made_in_time_is_a_timeout() {
        let error = Error::Timeout("connecting to the other device");
        assert_eq!(connect_failure(&error), WireError::TimedOut);
    }

    #[test]
    fn a_request_the_receiver_gave_up_waiting_for_is_asked_again() {
        assert_eq!(read_failure(&ReadFailure::TimedOut), WireError::TimedOut);
        assert_eq!(
            read_failure(&ReadFailure::Reset(close_code::TIMEOUT)),
            WireError::TimedOut
        );
    }

    #[test]
    fn a_request_the_receiver_took_for_a_violation_is_not_made_again() {
        assert_eq!(
            read_failure(&ReadFailure::Reset(close_code::PROTOCOL_VIOLATION)),
            WireError::Refused(close_code::PROTOCOL_VIOLATION)
        );
    }

    #[test]
    fn an_answer_that_cannot_be_read_is_not_asked_for_again() {
        for failure in [
            ReadFailure::BadHeader(FrameError::Empty),
            ReadFailure::BadPayload(FrameError::Truncated),
        ] {
            assert_eq!(
                read_failure(&failure),
                WireError::Refused(close_code::PROTOCOL_VIOLATION)
            );
        }
    }

    #[test]
    fn a_stream_that_was_abandoned_for_another_reason_is_a_connection_to_replace() {
        for reset in [close_code::CANCELLED, close_code::INTERNAL, 7] {
            assert_eq!(read_failure(&ReadFailure::Reset(reset)), WireError::Lost);
        }
        let cut = Error::protocol("the stream ended in the middle of a message");
        assert_eq!(read_failure(&ReadFailure::Transport(cut)), WireError::Lost);
    }
}

#[cfg(test)]
mod network_tests;
