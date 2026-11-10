// SPDX-License-Identifier: Apache-2.0

//! What the app does when a paired device asks what its person is working on, to continue it there
//! (`docs/adr/0015-continue-from-another-device.md`).
//!
//! The person at this device allowed the paired device beforehand, once, in the settings for it
//! ([`App::set_continue_access`](crate::App::set_continue_access)). When it asks, nobody here is
//! asked: the app tells the platform with [`Event::ContinueAsked`](crate::Event::ContinueAsked), which
//! reads the page that is in front in the browser and the text on the clipboard just then, and
//! answers with [`App::answer_continue`](crate::App::answer_continue). Nothing is read before the
//! asking, and nothing is kept after it.

use std::sync::Arc;

use baton_core::{ContinueHandler, PairedDevice, util::BoxFuture};
use baton_proto::message::{ContinueItems, DeclineReason};

use crate::app::Questions;

/// What reads what the person works on, for the node, through the platform.
pub(crate) struct Readings {
    questions: Arc<Questions>,
}

impl Readings {
    pub(crate) fn new(questions: Arc<Questions>) -> Self {
        Self { questions }
    }
}

impl ContinueHandler for Readings {
    fn items<'a>(
        &'a self,
        asker: &'a PairedDevice,
    ) -> BoxFuture<'a, Result<ContinueItems, DeclineReason>> {
        Box::pin(async move {
            // A platform that could not read it, a screen that is not there and a device that
            // stopped meanwhile are all a device that cannot give it just now.
            self.questions
                .ask_reading(asker.display_name())
                .await
                .ok_or(DeclineReason::Other)
        })
    }
}
