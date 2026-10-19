// SPDX-License-Identifier: Apache-2.0

//! Sending and receiving framed messages on QUIC streams.
//!
//! The framing itself lives in [`baton_proto::frame`]. This module adds the
//! I/O around it, with the two rules that matter for untrusted peers: nothing
//! is allocated before the declared length has been checked against the
//! message's limit, and every read has a deadline.

use std::time::Duration;

use baton_proto::{
    frame::{self, FrameError, HEADER_LEN},
    message::Message,
};
use iroh::endpoint::{
    ApplicationClose, ConnectionError, ReadError, ReadExactError, RecvStream, SendStream, VarInt,
    WriteError,
};

use crate::error::Error;

/// How the peer's bytes failed to become a message.
#[derive(Debug)]
pub(crate) enum ReadFailure {
    /// The length prefix is unacceptable: zero or over the limit. The receiver should reset the stream.
    BadHeader(FrameError),
    /// The payload could not be decoded or breaks a field limit. The receiver can still answer.
    BadPayload(FrameError),
    /// Nothing complete arrived in time.
    TimedOut,
    /// The other device abandoned the stream with this code.
    Reset(u32),
    /// The stream or connection failed.
    Transport(Error),
}

impl From<ReadFailure> for Error {
    fn from(failure: ReadFailure) -> Self {
        match failure {
            ReadFailure::BadHeader(e) | ReadFailure::BadPayload(e) => Self::protocol(&e),
            ReadFailure::TimedOut => Self::Timeout("waiting for the other device"),
            ReadFailure::Reset(code) => Self::Protocol(abandoned(code)),
            ReadFailure::Transport(e) => e,
        }
    }
}

/// Reads one message of type `M`, giving up after `deadline`.
pub(crate) async fn read_message<M: Message>(
    recv: &mut RecvStream,
    deadline: Duration,
) -> Result<M, ReadFailure> {
    match tokio::time::timeout(deadline, read_message_inner::<M>(recv)).await {
        Ok(result) => result,
        Err(_) => Err(ReadFailure::TimedOut),
    }
}

async fn read_message_inner<M: Message>(recv: &mut RecvStream) -> Result<M, ReadFailure> {
    let mut header = [0u8; HEADER_LEN];
    recv.read_exact(&mut header)
        .await
        .map_err(read_exact_failure)?;
    read_payload(recv, header).await
}

/// Reads the next message of a stream that carries many, one after the other, and waits as long as
/// it takes for it to begin. `None` means that the other end finished the stream between two
/// messages. Once the length of a message has arrived, the rest of it has `deadline`.
pub(crate) async fn read_next<M: Message>(
    recv: &mut RecvStream,
    deadline: Duration,
) -> Result<Option<M>, ReadFailure> {
    let mut header = [0u8; HEADER_LEN];
    match recv.read_exact(&mut header).await {
        Ok(()) => {}
        Err(ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(error) => return Err(read_exact_failure(error)),
    }
    match tokio::time::timeout(deadline, read_payload(recv, header)).await {
        Ok(result) => result.map(Some),
        Err(_) => Err(ReadFailure::TimedOut),
    }
}

/// Reads the payload of a message whose header has been read.
async fn read_payload<M: Message>(
    recv: &mut RecvStream,
    header: [u8; HEADER_LEN],
) -> Result<M, ReadFailure> {
    let len = frame::payload_len::<M>(header).map_err(ReadFailure::BadHeader)?;

    // Grow the buffer as bytes actually arrive instead of trusting the declared length.
    let mut payload = Vec::with_capacity(len.min(16 * 1024));
    while payload.len() < len {
        match recv.read_chunk(len - payload.len()).await {
            Ok(Some(chunk)) => payload.extend_from_slice(&chunk),
            Ok(None) => {
                return Err(ReadFailure::Transport(Error::protocol(
                    "the stream ended in the middle of a message",
                )));
            }
            Err(e) => return Err(read_failure(&e)),
        }
    }
    frame::decode_payload(&payload).map_err(ReadFailure::BadPayload)
}

/// Encodes `message` and writes it to `send`. The stream is left open.
pub(crate) async fn write_message<M: Message>(
    send: &mut SendStream,
    message: &M,
) -> Result<(), Error> {
    let bytes = frame::encode(message).map_err(|e| Error::protocol(&e))?;
    send.write_all(&bytes).await.map_err(|e| write_error(&e))
}

fn read_exact_failure(error: ReadExactError) -> ReadFailure {
    match error {
        ReadExactError::FinishedEarly(_) => ReadFailure::Transport(Error::protocol(
            "the stream ended before a message header arrived",
        )),
        ReadExactError::ReadError(e) => read_failure(&e),
    }
}

/// A failed read of a stream: the other device abandoned it, or the stream or connection failed.
fn read_failure(error: &ReadError) -> ReadFailure {
    match error {
        ReadError::Reset(code) => ReadFailure::Reset(code_of(*code)),
        other => ReadFailure::Transport(read_error(other)),
    }
}

pub(crate) fn read_error(error: &ReadError) -> Error {
    match error {
        ReadError::ConnectionLost(e) => connection_error("reading", e),
        ReadError::Reset(code) => Error::Protocol(abandoned(code_of(*code))),
        other => Error::network("reading", other),
    }
}

/// The code of a stop or a reset. No code of this protocol is larger than a `u32`, and one from a
/// device that sends a larger one has no meaning, so it is kept as large as it can be.
pub(crate) fn code_of(code: VarInt) -> u32 {
    u32::try_from(code.into_inner()).unwrap_or(u32::MAX)
}

fn abandoned(code: u32) -> String {
    format!("the other device abandoned the stream (code {code})")
}

pub(crate) fn write_error(error: &WriteError) -> Error {
    match error {
        WriteError::ConnectionLost(e) => connection_error("sending", e),
        WriteError::Stopped(code) => Error::Protocol(format!(
            "the other device stopped the stream (code {})",
            code.into_inner()
        )),
        other => Error::network("sending", other),
    }
}

/// Turns a connection failure into an [`Error`], keeping the peer's close code if it sent one.
pub(crate) fn connection_error(context: &'static str, error: &ConnectionError) -> Error {
    match error {
        ConnectionError::ApplicationClosed(ApplicationClose { error_code, reason }) => {
            Error::Closed {
                code: error_code.into_inner(),
                reason: String::from_utf8_lossy(reason).into_owned(),
            }
        }
        other => Error::network(context, other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_is_a_u32_whatever_the_other_device_sends() {
        assert_eq!(code_of(VarInt::from_u32(507)), 507);
        assert_eq!(code_of(VarInt::from_u32(0)), 0);
        let huge = VarInt::from_u64(u64::from(u32::MAX) + 5).unwrap();
        assert_eq!(code_of(huge), u32::MAX);
        assert_eq!(code_of(VarInt::from_u32(u32::MAX)), u32::MAX);
    }
}
