// SPDX-License-Identifier: Apache-2.0

//! The sessions that a paired device starts at this one (ADR 0010).
//!
//! The person at this device allowed the paired device beforehand, once, in the settings for it
//! ([`App::set_start_access`](crate::App::set_start_access)). When it asks, nobody here is asked:
//! the platform gives the main screen without a picker ([`WindowSharer::screen`]), and the app
//! offers it under the id that the paired device chose, with the place that the node took when it
//! asked. From then on the session is like any other that this device shares.

use std::sync::{Arc, OnceLock, Weak};

use baton_core::{
    Node, PairedDevice,
    streaming::{StartHandler, StartRequest, StartVerdict, Wanted, WhyNot},
    util::BoxFuture,
};

use crate::{model::WindowShared, sessions::Sessions, windows::ScreenError};

/// What the app does when a paired device that is allowed asks this one to start sharing its
/// screen.
pub(crate) struct Starts {
    sessions: Arc<Sessions>,
    /// The node that makes the offer. The handler is given to the node when it is built, so the
    /// node exists only after the handler does; it is held loosely, since the node holds the
    /// handler.
    node: OnceLock<Weak<Node>>,
}

impl Starts {
    pub(crate) fn new(sessions: Arc<Sessions>) -> Arc<Self> {
        Arc::new(Self {
            sessions,
            node: OnceLock::new(),
        })
    }

    /// The node is up: from now on a start can be offered with it.
    pub(crate) fn serve(&self, node: &Arc<Node>) {
        // Once, when the app starts. A second call would be a bug, and changes nothing.
        let _ = self.node.set(Arc::downgrade(node));
    }
}

impl ScreenError {
    /// What the device that asked is told.
    fn why(&self) -> WhyNot {
        match self {
            Self::Unsupported => WhyNot::Unsupported,
            Self::NeedsPermission => WhyNot::NeedsPermission,
            Self::Unavailable => WhyNot::Unavailable,
            Self::Failed(_) => WhyNot::Failed,
        }
    }
}

impl StartHandler for Starts {
    fn start<'a>(
        &'a self,
        from: &'a PairedDevice,
        request: StartRequest,
    ) -> BoxFuture<'a, StartVerdict> {
        Box::pin(async move {
            let StartRequest {
                wanted,
                control,
                slot,
                ..
            } = request;
            if wanted != Wanted::Screen {
                return StartVerdict::NotStarted(WhyNot::Unsupported);
            }
            let Some(sharer) = self
                .sessions
                .sharer()
                .filter(|sharer| sharer.can_give_screen())
            else {
                return StartVerdict::NotStarted(WhyNot::Unsupported);
            };
            let Some(node) = self.node.get().and_then(Weak::upgrade) else {
                // The node is not up yet, or is going down: the paired device may ask again.
                tracing::debug!("a screen was asked for while the node is not up");
                return StartVerdict::NotStarted(WhyNot::Failed);
            };
            let device = from.id.short();
            let picked = match sharer.screen().await {
                Ok(picked) if picked.display() => picked,
                Ok(_) => {
                    // The platform is set up wrongly: a window is not what was asked for, and is
                    // not offered in its place.
                    tracing::error!(%device, "the platform gave a window and not the screen");
                    return StartVerdict::NotStarted(WhyNot::Failed);
                }
                Err(error) => {
                    // What the platform said is a sentence for the person here, not for the log.
                    let why = error.why();
                    tracing::info!(%device, ?why, "the screen could not be given");
                    return StartVerdict::NotStarted(why);
                }
            };
            // The paired device gets no more control than it asked for and the settings allow
            // (the slot), and none if the platform cannot post input now: the node was set up when
            // it could, and the person may have taken the consent back since.
            let control = control && sharer.control();
            tracing::info!(%device, control, "a paired device started a screen");
            let sessions = Arc::clone(&self.sessions);
            let (peer, name) = (slot.peer(), from.display_name().to_owned());
            // The offer is made on a task of its own. Made here, the answer to the start would wait
            // until the session began, and the time that the application has to get ready would
            // have to cover that as well. The slot is let go of if the task never runs.
            tokio::spawn(async move {
                match sessions
                    .share(&node, &peer, &name, picked, control, Some(slot))
                    .await
                {
                    Ok(WindowShared::Started(_)) => {}
                    // The sentence names the device, and is not for the log.
                    Ok(_) => tracing::info!(%device, "the screen that was asked for was not taken"),
                    Err(error) => tracing::info!(
                        %device,
                        kind = ?error.kind(),
                        "the screen that was asked for was not offered"
                    ),
                }
            });
            StartVerdict::Starting
        })
    }
}
