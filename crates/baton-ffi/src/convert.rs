// SPDX-License-Identifier: Apache-2.0

//! From the values of `baton_app` to the ones the Dart side sees.
//!
//! The matches have no wildcard arms, and the structs are taken apart without `..`, on
//! purpose: a variant or a field added to `baton_app` stops this crate from compiling until
//! the bridge says what to do with it.

use baton_app as app;

use crate::api::{
    error::{BatonError, ErrorKind},
    model::{
        Device, Event, FileProgress, FilesSent, Me, Network, Offer, OfferOutcome, PairingRequest,
        Platform, PreviewName, Reach, Received, ReceivedItem, Receiving, Route, SendStage,
        TransferEnd, TransferRequest, TransferResult, WindowAbilities, WindowEnd, WindowRequest,
        WindowRole, WindowSession, WindowShared,
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
        let app::Me {
            id,
            name,
            platform,
            fingerprint,
        } = me;
        Self {
            id,
            name,
            platform: platform.into(),
            fingerprint,
        }
    }
}

impl From<app::Device> for Device {
    fn from(device: app::Device) -> Self {
        let app::Device {
            id,
            name,
            platform,
            fingerprint,
            paired_at,
            last_seen_at,
        } = device;
        Self {
            id,
            name,
            platform: platform.into(),
            fingerprint,
            paired_at: signed(paired_at),
            last_seen_at: last_seen_at.map(signed),
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
        let app::Reach {
            round_trip_ms,
            route,
        } = reach;
        Self {
            round_trip_ms,
            route: route.into(),
        }
    }
}

impl From<app::Network> for Network {
    fn from(network: app::Network) -> Self {
        let app::Network {
            relay,
            addresses,
            lan_discovery,
        } = network;
        Self {
            relay,
            addresses,
            lan_discovery,
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
        let app::Received {
            number,
            from_id,
            from_name,
            received_at,
            item,
        } = received;
        Self {
            number: signed(number),
            from_id,
            from_name,
            received_at: signed(received_at),
            item: item.into(),
        }
    }
}

impl From<app::Offer> for Offer {
    fn from(offer: app::Offer) -> Self {
        let app::Offer { ticket, expires_at } = offer;
        Self {
            ticket,
            expires_at: signed(expires_at),
        }
    }
}

impl From<app::PairingRequest> for PairingRequest {
    fn from(request: app::PairingRequest) -> Self {
        let app::PairingRequest {
            id,
            device_id,
            name,
            platform,
            fingerprint,
            code,
        } = request;
        Self {
            id: signed(id),
            device_id,
            name,
            platform: platform.into(),
            fingerprint,
            code,
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

impl From<app::PreviewName> for PreviewName {
    fn from(item: app::PreviewName) -> Self {
        let app::PreviewName {
            name,
            folder,
            files,
            bytes,
        } = item;
        Self {
            name,
            folder,
            files,
            bytes: signed(bytes),
        }
    }
}

impl From<app::WindowAbilities> for WindowAbilities {
    fn from(abilities: app::WindowAbilities) -> Self {
        let app::WindowAbilities {
            can_show,
            can_share,
            can_control,
        } = abilities;
        Self {
            can_show,
            can_share,
            can_control,
        }
    }
}

impl From<app::TransferRequest> for TransferRequest {
    fn from(request: app::TransferRequest) -> Self {
        let app::TransferRequest {
            id,
            device_id,
            device_name,
            items,
            files,
            folders,
            bytes,
            preview,
        } = request;
        Self {
            id: signed(id),
            device_id,
            device_name,
            items,
            files,
            folders,
            bytes: signed(bytes),
            preview: preview.into_iter().map(PreviewName::from).collect(),
        }
    }
}

impl From<app::WindowRequest> for WindowRequest {
    fn from(request: app::WindowRequest) -> Self {
        let app::WindowRequest {
            id,
            device_id,
            device_name,
            title,
            app,
            width,
            height,
            control,
        } = request;
        Self {
            id: signed(id),
            device_id,
            device_name,
            title,
            app,
            width,
            height,
            control,
        }
    }
}

impl From<app::TransferResult> for TransferResult {
    fn from(result: app::TransferResult) -> Self {
        match result {
            app::TransferResult::Received { failed } => Self::Received { failed },
            app::TransferResult::Dropped => Self::Dropped,
        }
    }
}

impl From<app::WindowRole> for WindowRole {
    fn from(role: app::WindowRole) -> Self {
        match role {
            app::WindowRole::Viewing => Self::Viewing,
            app::WindowRole::Sharing => Self::Sharing,
        }
    }
}

impl From<app::TransferEnd> for TransferEnd {
    fn from(end: app::TransferEnd) -> Self {
        let app::TransferEnd {
            request,
            device_id,
            device_name,
            files,
            bytes,
            preview,
            result,
        } = end;
        Self {
            request: signed(request),
            device_id,
            device_name,
            files,
            bytes: signed(bytes),
            preview: preview.into_iter().map(PreviewName::from).collect(),
            result: result.into(),
        }
    }
}

impl From<app::WindowSession> for WindowSession {
    fn from(session: app::WindowSession) -> Self {
        let app::WindowSession {
            id,
            role,
            device_id,
            device_name,
            title,
            app,
            width,
            height,
            control,
            screen,
        } = session;
        Self {
            id: signed(id),
            role: role.into(),
            device_id,
            device_name,
            title,
            app,
            width,
            height,
            control,
            screen,
        }
    }
}

impl From<app::Receiving> for Receiving {
    fn from(receiving: app::Receiving) -> Self {
        match receiving {
            app::Receiving::Off => Self::Off,
            app::Receiving::On => Self::On,
            app::Receiving::Unusable(message) => Self::Unusable { message },
        }
    }
}

impl From<app::WindowEnd> for WindowEnd {
    fn from(end: app::WindowEnd) -> Self {
        match end {
            app::WindowEnd::ThisDevice => Self::ThisDevice,
            app::WindowEnd::OtherDevice => Self::OtherDevice,
            app::WindowEnd::Lost => Self::Lost,
            app::WindowEnd::Failed(message) => Self::Failed { message },
        }
    }
}

impl From<app::SendStage> for SendStage {
    fn from(stage: app::SendStage) -> Self {
        match stage {
            app::SendStage::Connecting => Self::Connecting,
            app::SendStage::Asking => Self::Asking,
            app::SendStage::Sending => Self::Sending,
            app::SendStage::Finishing => Self::Finishing,
            app::SendStage::Waiting => Self::Waiting,
            app::SendStage::Over => Self::Over,
        }
    }
}

impl From<app::FileProgress> for FileProgress {
    fn from(progress: app::FileProgress) -> Self {
        let app::FileProgress {
            stage,
            attempt,
            files,
            bytes,
            sent,
        } = progress;
        Self {
            stage: stage.into(),
            attempt,
            files,
            bytes: signed(bytes),
            sent: signed(sent),
        }
    }
}

impl From<app::FilesSent> for FilesSent {
    fn from(sent: app::FilesSent) -> Self {
        let app::FilesSent {
            files,
            bytes,
            failed,
            left_out,
        } = sent;
        Self {
            files,
            bytes: signed(bytes),
            failed,
            left_out,
        }
    }
}

impl From<app::WindowShared> for WindowShared {
    fn from(shared: app::WindowShared) -> Self {
        match shared {
            app::WindowShared::NothingChosen => Self::NothingChosen,
            app::WindowShared::Started(session) => Self::Started {
                session: session.into(),
            },
            app::WindowShared::Declined(message) => Self::Declined { message },
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
            app::Event::TransferRequested(request) => Self::TransferRequested {
                request: request.into(),
            },
            app::Event::TransferRequestEnded { id } => {
                Self::TransferRequestEnded { id: signed(id) }
            }
            app::Event::TransferEnded(end) => Self::TransferEnded { end: end.into() },
            app::Event::WindowRequested(request) => Self::WindowRequested {
                request: request.into(),
            },
            app::Event::WindowRequestEnded { id, failure } => Self::WindowRequestEnded {
                id: signed(id),
                failure,
            },
            app::Event::WindowStarted(session) => Self::WindowStarted {
                session: session.into(),
            },
            app::Event::WindowEnded { id, end } => Self::WindowEnded {
                id: signed(id),
                end: end.into(),
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
            app::ErrorKind::Cancelled => Self::Cancelled,
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

    fn session() -> app::WindowSession {
        app::WindowSession {
            id: 4,
            role: app::WindowRole::Viewing,
            device_id: "ab".repeat(32),
            device_name: "Studio Mac".to_owned(),
            title: Some("Salary 2027.xlsx".to_owned()),
            app: Some("Numbers".to_owned()),
            width: 1440,
            height: 900,
            control: true,
            screen: Some(7),
        }
    }

    #[test]
    fn a_window_session_keeps_everything_it_says() {
        let converted = WindowSession::from(session());
        assert_eq!(converted.id, 4);
        assert_eq!(converted.role, WindowRole::Viewing);
        assert_eq!(converted.device_id, "ab".repeat(32));
        assert_eq!(converted.device_name, "Studio Mac");
        assert_eq!(converted.title.as_deref(), Some("Salary 2027.xlsx"));
        assert_eq!(converted.app.as_deref(), Some("Numbers"));
        assert_eq!((converted.width, converted.height), (1440, 900));
        assert!(converted.control);
        assert_eq!(converted.screen, Some(7));
        let sharing = WindowSession::from(app::WindowSession {
            role: app::WindowRole::Sharing,
            screen: None,
            ..session()
        });
        assert_eq!((sharing.role, sharing.screen), (WindowRole::Sharing, None));
    }

    #[test]
    fn a_window_request_keeps_everything_it_says() {
        let converted = WindowRequest::from(app::WindowRequest {
            id: 9,
            device_id: "cd".repeat(32),
            device_name: "Kitchen iPad".to_owned(),
            title: None,
            app: Some("Preview".to_owned()),
            width: 800,
            height: 600,
            control: false,
        });
        assert_eq!(converted.id, 9);
        assert_eq!(converted.device_id, "cd".repeat(32));
        assert_eq!(converted.device_name, "Kitchen iPad");
        assert_eq!(converted.title, None);
        assert_eq!(converted.app.as_deref(), Some("Preview"));
        assert_eq!((converted.width, converted.height), (800, 600));
        assert!(!converted.control);
    }

    #[test]
    fn the_window_events_keep_their_numbers_and_their_sentences() {
        let started = Event::from(app::Event::WindowStarted(session()));
        assert!(matches!(started, Event::WindowStarted { session } if session.id == 4));
        let refused = Event::from(app::Event::WindowRequestEnded {
            id: 3,
            failure: Some("The decoder is busy.".to_owned()),
        });
        assert!(matches!(
            refused,
            Event::WindowRequestEnded { id: 3, failure: Some(why) } if why == "The decoder is busy."
        ));
        let withdrawn = Event::from(app::Event::WindowRequestEnded {
            id: 3,
            failure: None,
        });
        assert!(matches!(
            withdrawn,
            Event::WindowRequestEnded {
                id: 3,
                failure: None
            }
        ));
        for (end, expected) in [
            (app::WindowEnd::ThisDevice, WindowEnd::ThisDevice),
            (app::WindowEnd::OtherDevice, WindowEnd::OtherDevice),
            (app::WindowEnd::Lost, WindowEnd::Lost),
            (
                app::WindowEnd::Failed("It broke.".to_owned()),
                WindowEnd::Failed {
                    message: "It broke.".to_owned(),
                },
            ),
        ] {
            let event = Event::from(app::Event::WindowEnded { id: 4, end });
            assert_eq!(
                event,
                Event::WindowEnded {
                    id: 4,
                    end: expected
                }
            );
        }
    }

    #[test]
    fn what_came_of_offering_a_window_keeps_its_session_and_its_sentence() {
        assert_eq!(
            WindowShared::from(app::WindowShared::NothingChosen),
            WindowShared::NothingChosen
        );
        assert!(matches!(
            WindowShared::from(app::WindowShared::Started(session())),
            WindowShared::Started { session } if session.id == 4
        ));
        assert_eq!(
            WindowShared::from(app::WindowShared::Declined("No.".to_owned())),
            WindowShared::Declined {
                message: "No.".to_owned()
            }
        );
    }

    #[test]
    fn what_a_device_can_do_with_windows_is_carried_over() {
        let converted = WindowAbilities::from(app::WindowAbilities {
            can_show: true,
            can_share: false,
            can_control: true,
        });
        assert_eq!(
            (
                converted.can_show,
                converted.can_share,
                converted.can_control
            ),
            (true, false, true)
        );
    }

    #[test]
    fn what_a_window_is_called_is_not_printed_on_the_rust_side() {
        let shown = format!(
            "{:?} {:?}",
            WindowSession::from(session()),
            WindowRequest::from(app::WindowRequest {
                id: 1,
                device_id: "ab".repeat(32),
                device_name: "Studio Mac".to_owned(),
                title: Some("Salary 2027.xlsx".to_owned()),
                app: Some("Numbers".to_owned()),
                width: 1,
                height: 1,
                control: false,
            })
        );
        assert!(!shown.contains("Salary"), "{shown}");
        assert!(!shown.contains("Numbers"), "{shown}");
    }

    #[test]
    fn a_request_for_files_keeps_everything_the_person_is_shown() {
        let request = app::TransferRequest {
            id: 7,
            device_id: "ab".repeat(32),
            device_name: "Kitchen iPad".to_owned(),
            items: 2,
            files: 5,
            folders: 3,
            bytes: 123_456_789_012,
            preview: vec![
                app::PreviewName {
                    name: "Holiday".to_owned(),
                    folder: true,
                    files: 4,
                    bytes: 123_000_000_000,
                },
                app::PreviewName {
                    name: "notes.txt".to_owned(),
                    folder: false,
                    files: 0,
                    bytes: 456_789_012,
                },
            ],
        };
        let converted = TransferRequest::from(request);
        assert_eq!(
            (converted.id, converted.device_name.as_str()),
            (7, "Kitchen iPad")
        );
        assert_eq!(converted.device_id, "ab".repeat(32));
        assert_eq!(
            (converted.items, converted.files, converted.folders),
            (2, 5, 3)
        );
        assert_eq!(converted.bytes, 123_456_789_012);
        assert_eq!(
            converted.preview,
            [
                PreviewName {
                    name: "Holiday".to_owned(),
                    folder: true,
                    files: 4,
                    bytes: 123_000_000_000,
                },
                PreviewName {
                    name: "notes.txt".to_owned(),
                    folder: false,
                    files: 0,
                    bytes: 456_789_012,
                },
            ]
        );
        // The names are shown to the person and are not for a log.
        assert!(!format!("{converted:?}").contains("Holiday"));
    }

    #[test]
    fn how_a_transfer_ended_and_how_far_it_got_are_copied_as_they_are() {
        let end = app::TransferEnd {
            request: 3,
            device_id: "cd".repeat(32),
            device_name: "Mac".to_owned(),
            files: 2,
            bytes: 99,
            preview: Vec::new(),
            result: app::TransferResult::Received { failed: 1 },
        };
        let converted = TransferEnd::from(end);
        assert_eq!(
            (converted.request, converted.files, converted.bytes),
            (3, 2, 99)
        );
        assert_eq!(converted.result, TransferResult::Received { failed: 1 });
        assert_eq!(
            TransferResult::from(app::TransferResult::Dropped),
            TransferResult::Dropped
        );

        for (stage, copy) in [
            (app::SendStage::Connecting, SendStage::Connecting),
            (app::SendStage::Asking, SendStage::Asking),
            (app::SendStage::Sending, SendStage::Sending),
            (app::SendStage::Finishing, SendStage::Finishing),
            (app::SendStage::Waiting, SendStage::Waiting),
            (app::SendStage::Over, SendStage::Over),
        ] {
            assert_eq!(SendStage::from(stage), copy, "{stage:?}");
        }
        let progress = FileProgress::from(app::FileProgress {
            stage: app::SendStage::Sending,
            attempt: 2,
            files: 3,
            bytes: 10_000_000_000,
            sent: 4_000_000_000,
        });
        assert_eq!(
            progress,
            FileProgress {
                stage: SendStage::Sending,
                attempt: 2,
                files: 3,
                bytes: 10_000_000_000,
                sent: 4_000_000_000,
            }
        );
        let sent = FilesSent::from(app::FilesSent {
            files: 3,
            bytes: 7,
            failed: 1,
            left_out: 2,
        });
        assert_eq!(
            sent,
            FilesSent {
                files: 3,
                bytes: 7,
                failed: 1,
                left_out: 2
            }
        );
        assert_eq!(
            Receiving::from(app::Receiving::Unusable("No.".to_owned())),
            Receiving::Unusable {
                message: "No.".to_owned()
            }
        );
        assert_eq!(Receiving::from(app::Receiving::Off), Receiving::Off);
        assert_eq!(Receiving::from(app::Receiving::On), Receiving::On);
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

    #[test]
    fn every_kind_of_error_is_copied_one_for_one() {
        for (kind, copy) in [
            (app::ErrorKind::AlreadyRunning, ErrorKind::AlreadyRunning),
            (app::ErrorKind::Stopped, ErrorKind::Stopped),
            (app::ErrorKind::Storage, ErrorKind::Storage),
            (app::ErrorKind::WrongMachine, ErrorKind::WrongMachine),
            (app::ErrorKind::Network, ErrorKind::Network),
            (app::ErrorKind::NotPaired, ErrorKind::NotPaired),
            (app::ErrorKind::Unreachable, ErrorKind::Unreachable),
            (app::ErrorKind::Declined, ErrorKind::Declined),
            (app::ErrorKind::BadItem, ErrorKind::BadItem),
            (app::ErrorKind::BadName, ErrorKind::BadName),
            (app::ErrorKind::BadTicket, ErrorKind::BadTicket),
            (app::ErrorKind::PairingFailed, ErrorKind::PairingFailed),
            (app::ErrorKind::Cancelled, ErrorKind::Cancelled),
            (app::ErrorKind::Other, ErrorKind::Other),
        ] {
            assert_eq!(ErrorKind::from(kind), copy, "{kind:?}");
        }
    }
}
