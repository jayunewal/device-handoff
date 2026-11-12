// SPDX-License-Identifier: Apache-2.0

//! What a device gives a paired device that asks what its person is working on, to continue it
//! there (`docs/protocol.md` section 3, Continue; `docs/adr/0015-continue-from-another-device.md`).
//!
//! The person at a device decides, for each paired device, whether it may ask
//! ([`DevicePolicy::continuing`](crate::DevicePolicy::continuing)), and nothing that a paired
//! device sends changes it. A device that may ask is given the page that is in front in the
//! browser here and the text on the clipboard, when it asks and not before: nothing is read in
//! advance, nothing is kept, and a device that is not asked is not read. Nothing shows that a
//! device was read for either, so it may ask only a few times a minute, fewer than the items that
//! it may send ([`HandoffConfig::max_continues_per_minute`]).
//!
//! [`HandoffConfig::max_continues_per_minute`]: crate::config::HandoffConfig::max_continues_per_minute

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use baton_proto::{
    DeviceId,
    message::{ContinueItems, ContinueReply, ContinueRequest, DeclineReason, Response},
};

use crate::{
    error::{Error, Result},
    trust::PairedDevice,
    util::BoxFuture,
};

/// What the application of a device that gives what its person works on does when a paired
/// device asks for it.
///
/// By the time [`items`](Self::items) is called, the device is paired, the request is
/// well-formed, the person here has allowed that device ([`DevicePolicy::continuing`]), it has
/// not asked more often than the limits for requests to continue and for items allow, and no
/// other question is waiting. A device that was given one advertises `continue.serve`.
///
/// [`DevicePolicy::continuing`]: crate::DevicePolicy::continuing
pub trait ContinueHandler: Send + Sync + 'static {
    /// What the person here is working on: the page that is in front in the browser, and the text
    /// on the clipboard. A part that there is none of is left out, and a device that is locked,
    /// or that is not being used, gives nothing at all rather than what it last held. Nothing is
    /// read that the person has marked as concealed, a password for example.
    ///
    /// The application may say [`Unsupported`](DeclineReason::Unsupported) if it cannot read such
    /// things at all, and [`Other`](DeclineReason::Other) if it could not just now. It has
    /// [`Tuning::continue_prepare`](crate::Tuning::continue_prepare) to answer, and the future is
    /// dropped when that is up. What it gives is held to the limits of a link and of clipboard
    /// text; a part that breaks them is left out.
    fn items<'a>(
        &'a self,
        asker: &'a PairedDevice,
    ) -> BoxFuture<'a, Result<ContinueItems, DeclineReason>>;
}

// An application that keeps a handle to its handler hands the node a clone of it.
impl<T: ContinueHandler + ?Sized> ContinueHandler for Arc<T> {
    fn items<'a>(
        &'a self,
        asker: &'a PairedDevice,
    ) -> BoxFuture<'a, Result<ContinueItems, DeclineReason>> {
        (**self).items(asker)
    }
}

/// What came of asking a paired device what its person is working on.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContinueOutcome {
    /// What the person there has ready. It may be nothing at all: the device is locked, or there
    /// is no page and no text.
    Items(ContinueItems),
    /// The device gave nothing, and says why. [`Blocked`](DeclineReason::Blocked) is its person not
    /// having allowed this device, which only they can change there.
    Declined(DeclineReason),
}

impl ContinueOutcome {
    /// What the response of a paired device to the request comes to.
    ///
    /// # Errors
    ///
    /// The response is not one to a request to continue ([`Error::Protocol`]).
    pub(crate) fn of(response: Response) -> Result<Self> {
        match response {
            Response::Continue(ContinueReply::Items(items)) => Ok(Self::Items(items)),
            Response::Continue(ContinueReply::Declined(reason)) => Ok(Self::Declined(reason)),
            _ => Err(Error::protocol(
                "the answer to a request to continue was not one to it",
            )),
        }
    }
}

/// The one question that a device answers at a time. It is let go of when this is dropped,
/// whatever ended the question, including the future being dropped.
struct Reading<'a>(&'a AtomicBool);

impl<'a> Reading<'a> {
    /// Takes the place, if it is free.
    fn begin(place: &'a AtomicBool) -> Option<Self> {
        (!place.swap(true, Ordering::SeqCst)).then_some(Self(place))
    }
}

impl Drop for Reading<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// What a node does when a paired device asks to continue its person's work: the handler it was
/// given, and the one question that it answers at a time.
pub(crate) struct Continuing {
    handler: Arc<dyn ContinueHandler>,
    reading: AtomicBool,
    prepare: Duration,
}

impl Continuing {
    pub(crate) fn new(handler: Arc<dyn ContinueHandler>, prepare: Duration) -> Self {
        Self {
            handler,
            reading: AtomicBool::new(false),
            prepare,
        }
    }

    /// Answers a request that `asker` made. `charge` says whether the device may make one more
    /// request of the kind that counts against the limits for requests to continue and for
    /// items.
    pub(crate) async fn answer(
        &self,
        asker: &PairedDevice,
        request: ContinueRequest,
        charge: impl Fn(&DeviceId) -> bool,
    ) -> ContinueReply {
        let ContinueRequest::Ask(_) = request;
        let declined = ContinueReply::Declined;
        // The person here decides for each paired device, and a device that was not allowed is
        // not read for, and does not count against the limit either.
        if !asker.policy.continuing {
            return declined(DeclineReason::Blocked);
        }
        if !charge(&asker.id) {
            return declined(DeclineReason::RateLimited);
        }
        // Reading what a person works on is not free, and one question at a time is all that
        // anybody needs.
        let Some(_place) = Reading::begin(&self.reading) else {
            return declined(DeclineReason::RateLimited);
        };
        match tokio::time::timeout(self.prepare, self.handler.items(asker)).await {
            Err(_) => declined(DeclineReason::Other),
            Ok(Err(reason)) => declined(of_the_ones_named(reason)),
            Ok(Ok(items)) => ContinueReply::Items(within_the_limits(items, asker)),
        }
    }
}

/// The reasons that the answer to a request to continue names. Another one, which the handler
/// has no business giving, is the one for a failure.
const fn of_the_ones_named(reason: DeclineReason) -> DeclineReason {
    match reason {
        DeclineReason::Blocked | DeclineReason::Unsupported | DeclineReason::RateLimited => reason,
        _ => DeclineReason::Other,
    }
}

/// What the handler gave, without any part that the protocol would not carry: a page with no
/// address or with one that is too long, text with nothing in it or too much. Such a part is left
/// out, and the other is given, since a device that sent nothing at all for one part that is wrong
/// would be of no use to the person.
fn within_the_limits(items: ContinueItems, asker: &PairedDevice) -> ContinueItems {
    let page = items.page.filter(|page| {
        let fits = page.validate().is_ok();
        if !fits {
            tracing::warn!(peer = %asker.id.short(), "the page that was to be given is not one that the protocol carries");
        }
        fits
    });
    let clipboard = items.clipboard.filter(|clipboard| {
        let fits = clipboard.validate().is_ok();
        if !fits {
            tracing::warn!(peer = %asker.id.short(), "the text that was to be given is not one that the protocol carries");
        }
        fits
    });
    ContinueItems { page, clipboard }
}
