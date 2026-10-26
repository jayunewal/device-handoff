// SPDX-License-Identifier: Apache-2.0

//! The messages of an unlock, as they travel on the control streams of a session.
//!
//! The design is in `docs/adr/0013-unlock-from-a-phone.md` and the wire format in section 9 of
//! `docs/protocol.md`. When the source of a private session locks, the viewer can ask it to
//! unlock, if the person at the source set that up for the viewer (the `unlock` of the offer).
//! The exchange has one question and one answer that carry secrets, and a few small messages
//! around them:
//!
//! 1. the viewer asks, with [`ViewerMessage::Unlock`](crate::control::ViewerMessage::Unlock);
//! 2. the source sends the key of its box, sealed to the viewer, and a challenge
//!    ([`Sealed`]), or says at once that it will not try ([`Done`]);
//! 3. the viewer has its person's face recognised, which opens the sealed key, and answers with
//!    the key and the challenge ([`Opened`]), or gives up ([`Cancelled`]);
//! 4. the source opens its box with the key, types the password, and says what came of it
//!    ([`Done`]).
//!
//! The key and the challenge are the only secrets. They are never printed: their `Debug` shows
//! nothing of them, so that a message can be logged whole.

use baton_proto::unlock::{BoxKey, Challenge, SealedKey};
use serde::{Deserialize, Serialize};

/// The source has made a question for the viewer to answer with its person's face.
///
/// It is the key of the source's box, sealed to a key that only the viewer can use, and the
/// challenge that the answer has to repeat. Only to a viewer that advertised
/// [`capability::UNLOCK_APPROVE`](baton_proto::message::capability::UNLOCK_APPROVE), in a session
/// whose offer set `unlock`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sealed {
    /// Made at random for this question and never used for another. The answer repeats it, so
    /// that an answer that comes late, or twice, answers nothing.
    pub challenge: Challenge,
    /// The key of the source's box, sealed to the key that the viewer gave when the unlock was set
    /// up. Only the viewer can open it, and only after its person's face is recognised.
    pub sealed: SealedKey,
}

/// The viewer has had its person's face recognised and opened the sealed key.
///
/// The key opens the box in which the source keeps the password, and nothing else. It travels
/// inside the encrypted connection, to the source that asked, and the viewer wipes its copy once
/// it is sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Opened {
    /// The challenge of the [`Sealed`] that this answers. The source drops an answer whose
    /// challenge it does not hold.
    pub challenge: Challenge,
    /// The key of the source's box.
    pub key: BoxKey,
}

/// Why the viewer gave up on a question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CancelReason {
    /// The person did not agree, their face was not recognised, or they cancelled.
    Declined,
    /// The viewer no longer has the key that opens the sealed one: a face was added or removed,
    /// the key was deleted, or the device was restored. Only the person at the source can set the
    /// unlock up again.
    KeyLost,
    /// Anything else that went wrong at the viewer, such as giving up waiting for the source.
    Failed,
    /// Any reason this version does not know about.
    #[serde(other)]
    Other,
}

/// The viewer gives up on the question it holds, or on the one it asked for. Nothing is opened
/// and no key follows.
///
/// A source that has no question outstanding ignores it. It answers nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cancelled {
    /// Why, so that the source can write it in its record.
    pub reason: CancelReason,
}

/// What came of an unlock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    /// The screen is unlocked.
    Unlocked,
    /// No unlock is set up for this viewer, or the one that was is no longer usable. The person
    /// has to set it up again at the source.
    NotSetUp,
    /// The screen is not locked, so nothing was typed.
    NotLocked,
    /// The screen of the source or its keys and pointer are not kept private at the moment, so
    /// nothing was typed. The viewer lifted the privacy, or the source cannot vouch for it.
    NotPrivate,
    /// Another unlock is going on.
    Busy,
    /// The key did not open the box. Nothing was typed, and the person has to set the unlock up
    /// again at the source.
    Stale,
    /// The password was typed and the screen stayed locked. The person has to set the unlock up
    /// again at the source.
    Rejected,
    /// The viewer did not answer in time.
    TimedOut,
    /// The source could not do it for a reason of its own, such as not being allowed to post keys.
    Failed,
    /// Any outcome this version does not know about.
    #[serde(other)]
    Other,
}

/// The source says what came of an unlock, or why it will not try.
///
/// It ends every unlock that the source took up, except one that the viewer gave up on, which the
/// viewer already knows the end of. It answers an [`Unlock`](crate::control::ViewerMessage::Unlock)
/// at once when the source will not try.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Done {
    /// What came of it.
    pub outcome: Outcome,
}
