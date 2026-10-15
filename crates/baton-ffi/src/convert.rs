// SPDX-License-Identifier: Apache-2.0

//! From the values of `baton_app` to the ones the Dart side sees.
//!
//! The matches have no wildcard arms on purpose: a variant added to `baton_app` stops this
//! crate from compiling until the bridge says what to do with it.

use baton_app as app;

use crate::api::{
    error::{BatonError, ErrorKind},
    model::{
        Device, Event, Me, Network, Offer, OfferOutcome, PairingRequest, Platform, Reach, Received,
        ReceivedItem, Route,
    },
};

/// A count or a time as the integer Dart has. Nothing here gets near the top of the range.
pub(crate) fn signed(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

impl From<app::Platform> for Platform {
    fn from(platform: app::Platform) -> Self {
        match platform {
            app::Platform::MacOs => Self::MacOs,
            app::Platform::Windows => Self::Windows,
            app::Platform::Linux => Self::Linux,
            app::Platform::Ios => Self::Ios,
            app::Platform::IpadOs => Self::IpadOs,
            app::Platform::Android => Self::Android,
            app::Platform::Other => Self::Other,
        }
    }
}

impl From<app::Me> for Me {
    fn from(me: app::Me) -> Self {
        Self {
            id: me.id,
            name: me.name,
            platform: me.platform.into(),
            fingerprint: me.fingerprint,
        }
    }
}

impl From<app::Device> for Device {
    fn from(device: app::Device) -> Self {
        Self {
            id: device.id,
            name: device.name,
            platform: device.platform.into(),
            fingerprint: device.fingerprint,
            paired_at: signed(device.paired_at),
            last_seen_at: device.last_seen_at.map(signed),
        }
    }
}

impl From<app::Route> for Route {
    fn from(route: app::Route) -> Self {
        match route {
            app::Route::Lan => Self::Lan,
            app::Route::Direct => Self::Direct,
            app::Route::Relay => Self::Relay,
            app::Route::Unknown => Self::Unknown,
        }
    }
}

impl From<app::Reach> for Reach {
    fn from(reach: app::Reach) -> Self {
        Self {
            round_trip_ms: reach.round_trip_ms,
            route: reach.route.into(),
        }
    }
}

impl From<app::Network> for Network {
    fn from(network: app::Network) -> Self {
        Self {
            relay: network.relay,
            addresses: network.addresses,
            lan_discovery: network.lan_discovery,
        }
    }
}

impl From<app::ReceivedItem> for ReceivedItem {
    fn from(item: app::ReceivedItem) -> Self {
        match item {
            app::ReceivedItem::Link { url, title } => Self::Link { url, title },
            app::ReceivedItem::Text { text } => Self::Text { text },
        }
    }
}

impl From<app::Received> for Received {
    fn from(received: app::Received) -> Self {
        Self {
            number: signed(received.number),
            from_id: received.from_id,
            from_name: received.from_name,
            received_at: signed(received.received_at),
            item: received.item.into(),
        }
    }
}

impl From<app::Offer> for Offer {
    fn from(offer: app::Offer) -> Self {
        Self {
            ticket: offer.ticket,
            expires_at: signed(offer.expires_at),
        }
    }
}

impl From<app::PairingRequest> for PairingRequest {
    fn from(request: app::PairingRequest) -> Self {
        Self {
            id: signed(request.id),
            device_id: request.device_id,
            name: request.name,
            platform: request.platform.into(),
            fingerprint: request.fingerprint,
            code: request.code,
        }
    }
}

impl From<app::OfferOutcome> for OfferOutcome {
    fn from(outcome: app::OfferOutcome) -> Self {
        match outcome {
            app::OfferOutcome::Paired(device) => Self::Paired {
                device: device.into(),
            },
            app::OfferOutcome::Expired => Self::Expired,
            app::OfferOutcome::Cancelled => Self::Cancelled,
            app::OfferOutcome::Declined => Self::Declined,
            app::OfferOutcome::Abandoned => Self::Abandoned,
            app::OfferOutcome::Failed(message) => Self::Failed { message },
        }
    }
}

impl From<app::Event> for Event {
    fn from(event: app::Event) -> Self {
        match event {
            app::Event::Received(received) => Self::Received {
                received: received.into(),
            },
            app::Event::DevicesChanged => Self::DevicesChanged,
            app::Event::PairingRequested(request) => Self::PairingRequested {
                request: request.into(),
            },
            app::Event::PairingRequestEnded { id } => Self::PairingRequestEnded { id: signed(id) },
            app::Event::OfferEnded(outcome) => Self::OfferEnded {
                outcome: outcome.into(),
            },
            app::Event::Resync => Self::Resync,
        }
    }
}

impl From<app::ErrorKind> for ErrorKind {
    fn from(kind: app::ErrorKind) -> Self {
        match kind {
            app::ErrorKind::AlreadyRunning => Self::AlreadyRunning,
            app::ErrorKind::Stopped => Self::Stopped,
            app::ErrorKind::Storage => Self::Storage,
            app::ErrorKind::WrongMachine => Self::WrongMachine,
            app::ErrorKind::Network => Self::Network,
            app::ErrorKind::NotPaired => Self::NotPaired,
            app::ErrorKind::Unreachable => Self::Unreachable,
            app::ErrorKind::Declined => Self::Declined,
            app::ErrorKind::BadItem => Self::BadItem,
            app::ErrorKind::BadName => Self::BadName,
            app::ErrorKind::BadTicket => Self::BadTicket,
            app::ErrorKind::PairingFailed => Self::PairingFailed,
            app::ErrorKind::Other => Self::Other,
        }
    }
}

impl From<app::Error> for BatonError {
    fn from(error: app::Error) -> Self {
        Self {
            kind: error.kind().into(),
            message: error.message().to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_become_the_integers_dart_has() {
        assert_eq!(signed(0), 0);
        assert_eq!(signed(1_700_000_000), 1_700_000_000);
        assert_eq!(signed(u64::MAX), i64::MAX);
    }

    #[test]
    fn an_error_keeps_its_kind_and_its_sentence() {
        let dir = tempfile::tempdir().unwrap();
        // A name with nothing visible in it is the easiest real failure to make.
        let error = app::settings::set_device_name(dir.path(), "  ").unwrap_err();
        let converted = BatonError::from(error.clone());
        assert_eq!(converted.kind, ErrorKind::BadName);
        assert_eq!(converted.message, error.message());
    }
}
