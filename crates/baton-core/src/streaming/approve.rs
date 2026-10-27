// SPDX-License-Identifier: Apache-2.0

//! What a viewer answers when a source asks it for the key that an unlock is sealed to
//! (`docs/protocol.md` section 9, `docs/adr/0013-unlock-from-a-phone.md`).
//!
//! The person sets the unlock up at the source, and by no message. To seal the key of the box the
//! source needs a public key of this device, whose private half cannot leave its hardware and works
//! only after the face of the person here is recognised. The application makes the pair for that
//! source, or finds the one it made, and the node gives the public half back.
//!
//! Nothing here is secret, but a pair is a thing that a person decided to make for one source, so
//! the application decides each time, and one question is asked at a time.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use baton_proto::{
    message::{DeclineReason, Response},
    unlock::{Sealing, UnlockKey, UnlockReply, UnlockRequest},
};

use super::Streaming;
use crate::{
    error::{Error, Result},
    trust::PairedDevice,
    unlock_record::check_sealing_key,
    util::BoxFuture,
};

/// What the application of a device that unlocks others with the face of its person does when a
/// paired device asks it for the key that an unlock is sealed to.
///
/// By the time [`key`](Self::key) is called, the device is paired, the request is well-formed and
/// the device has not sent more items than it may. A device that was given one advertises
/// `unlock.approve`.
pub trait UnlockApprover: Send + Sync + 'static {
    /// The public key of the pair that this device made for `source`, and a new pair if it has
    /// none. With `renew` it replaces the pair first, and what was sealed to the old key opens
    /// nothing from then on: the person at the source set the unlock up again.
    ///
    /// The private half stays in hardware that opens it only for the face of the person here, and
    /// a face that is added or removed ends it. The key is of the kind that
    /// [`Sealing::EciesP256`](baton_proto::unlock::Sealing::EciesP256) names, a point of P-256 in
    /// the uncompressed form; the node refuses to pass on any other.
    ///
    /// The application may ask the person first. It says [`Refused`](DeclineReason::Refused) if
    /// they say no, [`Blocked`](DeclineReason::Blocked) if its settings do not let `source` ask,
    /// and [`Unsupported`](DeclineReason::Unsupported) if it cannot make such a pair. It has the
    /// time that a person has for an approval, and the future is dropped when that is up.
    fn key<'a>(
        &'a self,
        source: &'a PairedDevice,
        renew: bool,
    ) -> BoxFuture<'a, Result<UnlockKey, DeclineReason>>;
}

// An application that keeps a handle to its approver hands the node a clone of it.
impl<T: UnlockApprover + ?Sized> UnlockApprover for Arc<T> {
    fn key<'a>(
        &'a self,
        source: &'a PairedDevice,
        renew: bool,
    ) -> BoxFuture<'a, Result<UnlockKey, DeclineReason>> {
        (**self).key(source, renew)
    }
}

/// What came of asking a paired device for the key that an unlock is sealed to.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeyOutcome {
    /// The device gave the key. The platform seals the key of its box to it, and the person sets
    /// the unlock up with [`TrustStore::set_up_unlock`](crate::TrustStore::set_up_unlock).
    Given(UnlockKey),
    /// The device will not give one, and says why. A `Refused` is the person there saying no.
    Declined(DeclineReason),
}

impl KeyOutcome {
    /// What the response of a paired device to a request for a key comes to. The key it gives has
    /// to be one that the platform can seal to.
    ///
    /// # Errors
    ///
    /// The key is of a kind that this version does not know ([`Error::Unsupported`]), or is not a
    /// key of the kind it names, or the response is not one to a request for a key
    /// ([`Error::Protocol`]).
    pub(crate) fn of(response: Response) -> Result<Self> {
        match response {
            Response::Unlock(UnlockReply::Key(key)) => {
                if key.sealing == Sealing::Other {
                    return Err(Error::Unsupported(
                        "a way of sealing that this version does not know".to_owned(),
                    ));
                }
                check_sealing_key(key.sealing, &key.public).map_err(Error::protocol)?;
                Ok(Self::Given(key))
            }
            Response::Unlock(UnlockReply::Declined(reason)) => Ok(Self::Declined(reason)),
            _ => Err(Error::protocol(
                "the answer to a request for a key was not one to it",
            )),
        }
    }
}

/// The one question that a viewer asks at a time. It is let go of when this is dropped, whatever
/// ended the question, including the future being dropped.
struct Keying<'a>(&'a AtomicBool);

impl<'a> Keying<'a> {
    /// Takes the place, if it is free.
    fn begin(place: &'a AtomicBool) -> Option<Self> {
        (!place.swap(true, Ordering::SeqCst)).then_some(Self(place))
    }
}

impl Drop for Keying<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// The place of the one question about a key that a viewer has.
#[derive(Default)]
pub(super) struct KeyPlace(AtomicBool);

impl Streaming {
    /// Whether this device unlocks others with the face of its person, which is whether it
    /// advertises `unlock.approve`.
    pub(crate) fn approves_unlock(&self) -> bool {
        self.viewer
            .as_ref()
            .is_some_and(|viewer| viewer.approver.is_some())
    }

    /// Answers a request about unlocking that `paired` made. A device that was given nobody to
    /// ask has no key to give, and declines it as unsupported (section 9).
    pub(crate) async fn answer_unlock(
        &self,
        paired: &PairedDevice,
        request: UnlockRequest,
    ) -> UnlockReply {
        let UnlockRequest::Key(key) = request;
        let declined = UnlockReply::Declined;
        let Some((viewer, approver)) = self
            .viewer
            .as_ref()
            .and_then(|viewer| Some((viewer, viewer.approver.as_ref()?)))
        else {
            return declined(DeclineReason::Unsupported);
        };
        // Too many items from the device, or another question about a key that waits for the
        // person.
        if !(viewer.charge)(&paired.id) {
            return declined(DeclineReason::RateLimited);
        }
        let Some(_place) = Keying::begin(&viewer.keying.0) else {
            return declined(DeclineReason::RateLimited);
        };
        let answered = tokio::time::timeout(viewer.approval, approver.key(paired, key.renew)).await;
        match answered {
            Err(_) => declined(DeclineReason::Refused),
            Ok(Err(DeclineReason::Other)) => declined(DeclineReason::Unsupported),
            Ok(Err(reason)) => declined(reason),
            Ok(Ok(given)) => match check_sealing_key(given.sealing, &given.public) {
                Ok(()) => UnlockReply::Key(given),
                Err(why) => {
                    tracing::warn!(peer = %paired.id.short(), why, "the approver gave a key that cannot be passed on");
                    declined(DeclineReason::Unsupported)
                }
            },
        }
    }
}
