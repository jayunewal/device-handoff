// SPDX-License-Identifier: Apache-2.0

//! The state every part of a running node shares.

use std::{fmt, sync::Arc};

use baton_proto::{DeviceId, DeviceInfo};
use tokio_util::sync::CancellationToken;

use crate::{
    clock::Clock,
    gate::ConnectionRegistry,
    node::Tuning,
    pairing::Offers,
    session::{HandoffHandler, Intake},
    slots::PairSlots,
    trust::TrustStore,
};

pub(crate) struct Shared {
    /// This device.
    pub(crate) id: DeviceId,
    /// What this device tells others about itself.
    pub(crate) info: DeviceInfo,
    pub(crate) trust: TrustStore,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) tuning: Tuning,
    /// Whether any relay is configured. Without one, relay hints are useless.
    pub(crate) relay_enabled: bool,
    /// Cancelled when the node shuts down.
    pub(crate) stop: CancellationToken,
    pub(crate) registry: Arc<ConnectionRegistry>,
    pub(crate) offers: Offers,
    pub(crate) pair_slots: Arc<PairSlots>,
    pub(crate) handler: Arc<dyn HandoffHandler>,
    pub(crate) intake: Intake,
}

impl fmt::Debug for Shared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Shared")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
