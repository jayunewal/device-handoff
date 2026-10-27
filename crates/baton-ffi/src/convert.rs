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
        Platform, PreviewName, Reach, Received, ReceivedItem, Receiving, Route, ScreenStarted,
        SendStage, StartAccess, TransferEnd, TransferRequest, TransferResult, UnlockEntry,
        UnlockResult, UnlockSetUp, UnlockStatus, WindowAbilities, WindowButton, WindowEnd,
        WindowInput, WindowModifiers, WindowNoticeReason, WindowPointer, WindowRequest, WindowRole,
        WindowSession, WindowShared, WindowSound, WindowUnlock,
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
            can_give_screen,
            can_be_unlocked,
            can_unlock,
        } = abilities;
        Self {
            can_show,
            can_share,
            can_control,
            can_give_screen,
            can_be_unlocked,
            can_unlock,
        }
    }
}

impl From<app::StartAccess> for StartAccess {
    fn from(access: app::StartAccess) -> Self {
        match access {
            app::StartAccess::Off => Self::Off,
            app::StartAccess::Look => Self::Look,
            app::StartAccess::Control => Self::Control,
        }
    }
}

impl From<StartAccess> for app::StartAccess {
    fn from(access: StartAccess) -> Self {
        match access {
            StartAccess::Off => Self::Off,
            StartAccess::Look => Self::Look,
            StartAccess::Control => Self::Control,
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
            display,
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
            display,
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
            saved,
            result,
        } = end;
        Self {
            request: signed(request),
            device_id,
            device_name,
            files,
            bytes: signed(bytes),
            preview: preview.into_iter().map(PreviewName::from).collect(),
            saved: saved
                .into_iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
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
            display,
            title,
            app,
            width,
            height,
            control,
            private,
            sound,
            leave,
            unlock,
            screen,
        } = session;
        Self {
            id: signed(id),
            role: role.into(),
            device_id,
            device_name,
            display,
            title,
            app,
            width,
            height,
            control,
            private,
            sound,
            leave,
            unlock,
            screen,
        }
    }
}

impl From<app::WindowSound> for WindowSound {
    fn from(sound: app::WindowSound) -> Self {
        match sound {
            app::WindowSound::Playing => Self::Playing,
            app::WindowSound::Off => Self::Off,
            app::WindowSound::Window => Self::Window,
            app::WindowSound::NoPermission => Self::NoPermission,
            app::WindowSound::Failed => Self::Failed,
            app::WindowSound::Other => Self::Other,
            app::WindowSound::CannotPlay => Self::CannotPlay,
            app::WindowSound::NotAllowed => Self::NotAllowed,
        }
    }
}

impl From<app::UnlockResult> for UnlockResult {
    fn from(result: app::UnlockResult) -> Self {
        match result {
            app::UnlockResult::Unlocked => Self::Unlocked,
            app::UnlockResult::NotSetUp => Self::NotSetUp,
            app::UnlockResult::NotLocked => Self::NotLocked,
            app::UnlockResult::NotPrivate => Self::NotPrivate,
            app::UnlockResult::Busy => Self::Busy,
            app::UnlockResult::Stale => Self::Stale,
            app::UnlockResult::Rejected => Self::Rejected,
            app::UnlockResult::TimedOut => Self::TimedOut,
            app::UnlockResult::Failed => Self::Failed,
            app::UnlockResult::Other => Self::Other,
            app::UnlockResult::Declined => Self::Declined,
            app::UnlockResult::KeyLost => Self::KeyLost,
            app::UnlockResult::GaveUp => Self::GaveUp,
            app::UnlockResult::NoQuestion => Self::NoQuestion,
            app::UnlockResult::NoResult => Self::NoResult,
            app::UnlockResult::SessionEnded => Self::SessionEnded,
        }
    }
}

impl From<app::WindowUnlock> for WindowUnlock {
    fn from(step: app::WindowUnlock) -> Self {
        match step {
            app::WindowUnlock::Asking => Self::Asking,
            app::WindowUnlock::Typing => Self::Typing,
            app::WindowUnlock::Ended(result) => Self::Ended {
                result: result.into(),
            },
        }
    }
}

impl From<app::UnlockStatus> for UnlockStatus {
    fn from(status: app::UnlockStatus) -> Self {
        match status {
            app::UnlockStatus::Off => Self::Off,
            app::UnlockStatus::On { since } => Self::On {
                since: signed(since),
            },
            app::UnlockStatus::Ended { since } => Self::Ended {
                since: signed(since),
            },
        }
    }
}

impl From<app::UnlockSetUp> for UnlockSetUp {
    fn from(set_up: app::UnlockSetUp) -> Self {
        match set_up {
            app::UnlockSetUp::Done => Self::Done,
            app::UnlockSetUp::WrongPassword => Self::WrongPassword,
            app::UnlockSetUp::NotDone(message) => Self::NotDone { message },
        }
    }
}

impl From<app::UnlockEntry> for UnlockEntry {
    fn from(entry: app::UnlockEntry) -> Self {
        let app::UnlockEntry {
            at,
            device_id,
            device_name,
            result,
        } = entry;
        Self {
            at: signed(at),
            device_id,
            device_name,
            // The rule is the app's, so that a screen does not hold a copy of it.
            notify: result.notifies(),
            result: result.into(),
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

impl From<WindowButton> for app::WindowButton {
    fn from(button: WindowButton) -> Self {
        match button {
            WindowButton::Left => Self::Left,
            WindowButton::Right => Self::Right,
            WindowButton::Middle => Self::Middle,
        }
    }
}

impl From<WindowPointer> for app::WindowPointer {
    fn from(action: WindowPointer) -> Self {
        match action {
            WindowPointer::Move => Self::Move,
            WindowPointer::Down { button } => Self::Down(button.into()),
            WindowPointer::Up { button } => Self::Up(button.into()),
            WindowPointer::Scroll { dx, dy } => Self::Scroll { dx, dy },
        }
    }
}

impl From<WindowModifiers> for app::WindowModifiers {
    fn from(keys: WindowModifiers) -> Self {
        let WindowModifiers {
            shift,
            control,
            option,
            command,
            caps_lock,
        } = keys;
        Self {
            shift,
            control,
            option,
            command,
            caps_lock,
        }
    }
}

impl From<WindowInput> for app::WindowInput {
    fn from(input: WindowInput) -> Self {
        match input {
            WindowInput::Pointer {
                x,
                y,
                modifiers,
                action,
            } => Self::Pointer {
                x,
                y,
                modifiers: modifiers.into(),
                action: action.into(),
            },
            WindowInput::Key {
                code,
                down,
                modifiers,
                repeat,
                text,
            } => Self::Key {
                code,
                down,
                modifiers: modifiers.into(),
                repeat,
                text,
            },
        }
    }
}

impl From<app::WindowNoticeReason> for WindowNoticeReason {
    fn from(reason: app::WindowNoticeReason) -> Self {
        match reason {
            app::WindowNoticeReason::ControlOff => Self::ControlOff,
            app::WindowNoticeReason::Paused => Self::Paused,
            app::WindowNoticeReason::Locked => Self::Locked,
            app::WindowNoticeReason::NotOnScreen => Self::NotOnScreen,
            app::WindowNoticeReason::TooFast => Self::TooFast,
            app::WindowNoticeReason::OutOfDate => Self::OutOfDate,
            app::WindowNoticeReason::PictureUnknown => Self::PictureUnknown,
            app::WindowNoticeReason::Resized => Self::Resized,
            app::WindowNoticeReason::OutsideWindow => Self::OutsideWindow,
            app::WindowNoticeReason::Covered => Self::Covered,
            app::WindowNoticeReason::NoKeyboard => Self::NoKeyboard,
            app::WindowNoticeReason::AppWindowInFront => Self::AppWindowInFront,
            app::WindowNoticeReason::TooManyKeys => Self::TooManyKeys,
            app::WindowNoticeReason::NotPressed => Self::NotPressed,
            app::WindowNoticeReason::Other => Self::Other,
        }
    }
}

impl From<app::ScreenStarted> for ScreenStarted {
    fn from(started: app::ScreenStarted) -> Self {
        match started {
            app::ScreenStarted::Started(session) => Self::Started {
                session: session.into(),
            },
            app::ScreenStarted::NotStarted(message) => Self::NotStarted { message },
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
            app::Event::WindowPicture { id, width, height } => Self::WindowPicture {
                id: signed(id),
                width,
                height,
            },
            app::Event::WindowNotice { id, reason, count } => Self::WindowNotice {
                id: signed(id),
                reason: reason.into(),
                count,
            },
            app::Event::WindowSound { id, sound } => Self::WindowSound {
                id: signed(id),
                sound: sound.into(),
            },
            app::Event::WindowLocked { id, locked } => Self::WindowLocked {
                id: signed(id),
                locked,
            },
            app::Event::WindowUnlock { id, step } => Self::WindowUnlock {
                id: signed(id),
                step: step.into(),
            },
            app::Event::UnlockAttempted(entry) => Self::UnlockAttempted {
                entry: entry.into(),
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
            display: false,
            title: Some("Salary 2027.xlsx".to_owned()),
            app: Some("Numbers".to_owned()),
            width: 1440,
            height: 900,
            control: true,
            private: true,
            sound: true,
            leave: true,
            screen: Some(7),
            unlock: true,
        }
    }

    #[test]
    fn a_window_session_keeps_everything_it_says() {
        let converted = WindowSession::from(session());
        assert_eq!(converted.id, 4);
        assert_eq!(converted.role, WindowRole::Viewing);
        assert_eq!(converted.device_id, "ab".repeat(32));
        assert_eq!(converted.device_name, "Studio Mac");
        assert!(!converted.display);
        assert_eq!(converted.title.as_deref(), Some("Salary 2027.xlsx"));
        assert_eq!(converted.app.as_deref(), Some("Numbers"));
        assert_eq!((converted.width, converted.height), (1440, 900));
        assert!(converted.control);
        assert!(converted.private);
        assert!(converted.sound);
        assert!(converted.leave);
        assert!(converted.unlock);
        assert_eq!(converted.screen, Some(7));
        let sharing = WindowSession::from(app::WindowSession {
            role: app::WindowRole::Sharing,
            screen: None,
            ..session()
        });
        assert_eq!((sharing.role, sharing.screen), (WindowRole::Sharing, None));
        // The facts are each of their own: one that is off leaves the others as they were.
        let stays = WindowSession::from(app::WindowSession {
            leave: false,
            ..session()
        });
        assert!(!stays.leave && stays.private && stays.sound && stays.unlock);
        let locked_out = WindowSession::from(app::WindowSession {
            unlock: false,
            ..session()
        });
        assert!(!locked_out.unlock && locked_out.leave && locked_out.private && locked_out.sound);
        let display = WindowSession::from(app::WindowSession {
            display: true,
            ..session()
        });
        assert!(display.display);
    }

    #[test]
    fn a_window_request_keeps_everything_it_says() {
        let converted = WindowRequest::from(app::WindowRequest {
            id: 9,
            device_id: "cd".repeat(32),
            device_name: "Kitchen iPad".to_owned(),
            display: false,
            title: None,
            app: Some("Preview".to_owned()),
            width: 800,
            height: 600,
            control: false,
        });
        assert_eq!(converted.id, 9);
        assert_eq!(converted.device_id, "cd".repeat(32));
        assert_eq!(converted.device_name, "Kitchen iPad");
        assert!(!converted.display);
        assert_eq!(converted.title, None);
        assert_eq!(converted.app.as_deref(), Some("Preview"));
        assert_eq!((converted.width, converted.height), (800, 600));
        assert!(!converted.control);
        let display = WindowRequest::from(app::WindowRequest {
            id: 10,
            device_id: "cd".repeat(32),
            device_name: "Kitchen iPad".to_owned(),
            display: true,
            title: None,
            app: None,
            width: 3024,
            height: 1964,
            control: false,
        });
        assert!(display.display);
    }

    #[test]
    fn the_window_events_keep_their_numbers_and_their_sentences() {
        let started = Event::from(app::Event::WindowStarted(session()));
        assert!(matches!(started, Event::WindowStarted { session } if session.id == 4));
        let picture = Event::from(app::Event::WindowPicture {
            id: 4,
            width: 1920,
            height: 1080,
        });
        assert!(matches!(
            picture,
            Event::WindowPicture {
                id: 4,
                width: 1920,
                height: 1080
            }
        ));
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

    // Every result of an attempt, with what Dart sees. A result added to `baton_app` stops this
    // crate from compiling at the conversion, and the list below is where its mate is named.
    const RESULTS: [(app::UnlockResult, UnlockResult); 16] = [
        (app::UnlockResult::Unlocked, UnlockResult::Unlocked),
        (app::UnlockResult::NotSetUp, UnlockResult::NotSetUp),
        (app::UnlockResult::NotLocked, UnlockResult::NotLocked),
        (app::UnlockResult::NotPrivate, UnlockResult::NotPrivate),
        (app::UnlockResult::Busy, UnlockResult::Busy),
        (app::UnlockResult::Stale, UnlockResult::Stale),
        (app::UnlockResult::Rejected, UnlockResult::Rejected),
        (app::UnlockResult::TimedOut, UnlockResult::TimedOut),
        (app::UnlockResult::Failed, UnlockResult::Failed),
        (app::UnlockResult::Other, UnlockResult::Other),
        (app::UnlockResult::Declined, UnlockResult::Declined),
        (app::UnlockResult::KeyLost, UnlockResult::KeyLost),
        (app::UnlockResult::GaveUp, UnlockResult::GaveUp),
        (app::UnlockResult::NoQuestion, UnlockResult::NoQuestion),
        (app::UnlockResult::NoResult, UnlockResult::NoResult),
        (app::UnlockResult::SessionEnded, UnlockResult::SessionEnded),
    ];

    #[test]
    fn each_result_of_an_unlock_is_carried_over_as_itself() {
        for (result, shown) in RESULTS {
            assert_eq!(UnlockResult::from(result), shown, "{result:?}");
        }
        // No two of them are the same one: a person is told something different for each.
        for (index, (_, shown)) in RESULTS.iter().enumerate() {
            for (_, other) in &RESULTS[index + 1..] {
                assert_ne!(shown, other);
            }
        }
    }

    #[test]
    fn a_screen_that_locks_and_unlocks_is_told_to_dart_with_its_number() {
        for locked in [true, false] {
            assert_eq!(
                Event::from(app::Event::WindowLocked { id: 4, locked }),
                Event::WindowLocked { id: 4, locked }
            );
        }
        assert!(matches!(
            Event::from(app::Event::WindowLocked {
                id: u64::MAX,
                locked: true
            }),
            Event::WindowLocked {
                id: i64::MAX,
                locked: true
            }
        ));
    }

    #[test]
    fn each_step_of_an_unlock_is_told_to_dart_with_its_number() {
        let steps = [
            (app::WindowUnlock::Asking, WindowUnlock::Asking),
            (app::WindowUnlock::Typing, WindowUnlock::Typing),
        ];
        for (step, shown) in steps {
            assert_eq!(
                Event::from(app::Event::WindowUnlock { id: 4, step }),
                Event::WindowUnlock { id: 4, step: shown }
            );
        }
        for (result, shown) in RESULTS {
            assert_eq!(
                Event::from(app::Event::WindowUnlock {
                    id: 4,
                    step: app::WindowUnlock::Ended(result),
                }),
                Event::WindowUnlock {
                    id: 4,
                    step: WindowUnlock::Ended { result: shown },
                },
                "{result:?}"
            );
        }
    }

    #[test]
    fn an_attempt_to_unlock_this_device_says_whether_to_tell_the_person() {
        for (result, shown) in RESULTS {
            let entry = app::UnlockEntry {
                at: 1_700_000_000,
                device_id: "ab".repeat(32),
                device_name: "Pocket Phone".to_owned(),
                result,
            };
            let told = Event::from(app::Event::UnlockAttempted(entry.clone()));
            let Event::UnlockAttempted { entry: seen } = told else {
                panic!("{told:?}");
            };
            assert_eq!(seen.at, 1_700_000_000);
            assert_eq!(seen.device_id, "ab".repeat(32));
            assert_eq!(seen.device_name, "Pocket Phone");
            assert_eq!(seen.result, shown);
            // The rule is the app's, and the history says the same for the same attempt.
            assert_eq!(seen.notify, result.notifies(), "{result:?}");
            assert_eq!(UnlockEntry::from(entry), seen);
        }
        // The ones a person is told of, so that a change of the rule is seen here too.
        let told: Vec<_> = RESULTS
            .iter()
            .filter(|(result, _)| result.notifies())
            .map(|(_, shown)| *shown)
            .collect();
        assert_eq!(
            told,
            [
                UnlockResult::Unlocked,
                UnlockResult::Stale,
                UnlockResult::Rejected,
                UnlockResult::TimedOut,
                UnlockResult::Failed,
                UnlockResult::Other,
                UnlockResult::KeyLost,
                UnlockResult::GaveUp,
            ]
        );
    }

    #[test]
    fn a_time_that_is_too_large_for_dart_is_the_largest_it_has() {
        let entry = UnlockEntry::from(app::UnlockEntry {
            at: u64::MAX,
            device_id: "ab".repeat(32),
            device_name: "Pocket Phone".to_owned(),
            result: app::UnlockResult::Unlocked,
        });
        assert_eq!(entry.at, i64::MAX);
        assert_eq!(
            UnlockStatus::from(app::UnlockStatus::On { since: u64::MAX }),
            UnlockStatus::On { since: i64::MAX }
        );
    }

    #[test]
    fn where_the_unlock_of_a_device_stands_is_carried_over() {
        assert_eq!(
            UnlockStatus::from(app::UnlockStatus::Off),
            UnlockStatus::Off
        );
        assert_eq!(
            UnlockStatus::from(app::UnlockStatus::On {
                since: 1_700_000_000
            }),
            UnlockStatus::On {
                since: 1_700_000_000
            }
        );
        assert_eq!(
            UnlockStatus::from(app::UnlockStatus::Ended { since: 5 }),
            UnlockStatus::Ended { since: 5 }
        );
    }

    #[test]
    fn what_came_of_setting_the_unlock_up_keeps_its_sentence() {
        assert_eq!(UnlockSetUp::from(app::UnlockSetUp::Done), UnlockSetUp::Done);
        assert_eq!(
            UnlockSetUp::from(app::UnlockSetUp::WrongPassword),
            UnlockSetUp::WrongPassword
        );
        assert_eq!(
            UnlockSetUp::from(app::UnlockSetUp::NotDone(
                "Pocket Phone has no face to ask for.".to_owned()
            )),
            UnlockSetUp::NotDone {
                message: "Pocket Phone has no face to ask for.".to_owned()
            }
        );
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
            can_give_screen: false,
            can_be_unlocked: true,
            can_unlock: false,
        });
        assert_eq!(
            (
                converted.can_show,
                converted.can_share,
                converted.can_control,
                converted.can_give_screen,
                converted.can_be_unlocked,
                converted.can_unlock
            ),
            (true, false, true, false, true, false)
        );
        let converted = WindowAbilities::from(app::WindowAbilities {
            can_show: false,
            can_share: true,
            can_control: false,
            can_give_screen: true,
            can_be_unlocked: false,
            can_unlock: true,
        });
        assert_eq!(
            (
                converted.can_show,
                converted.can_share,
                converted.can_control,
                converted.can_give_screen,
                converted.can_be_unlocked,
                converted.can_unlock
            ),
            (false, true, false, true, false, true)
        );
    }

    #[test]
    fn what_a_paired_device_may_start_is_carried_over_both_ways() {
        for (access, shown) in [
            (app::StartAccess::Off, StartAccess::Off),
            (app::StartAccess::Look, StartAccess::Look),
            (app::StartAccess::Control, StartAccess::Control),
        ] {
            assert_eq!(StartAccess::from(access), shown);
            assert_eq!(app::StartAccess::from(shown), access);
        }
    }

    #[test]
    fn what_came_of_asking_for_a_screen_keeps_its_session_and_its_sentence() {
        assert!(matches!(
            ScreenStarted::from(app::ScreenStarted::Started(session())),
            ScreenStarted::Started { session } if session.id == 4
        ));
        assert_eq!(
            ScreenStarted::from(app::ScreenStarted::NotStarted("No.".to_owned())),
            ScreenStarted::NotStarted {
                message: "No.".to_owned()
            }
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
                display: false,
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

    fn keys_and_their_places() -> (WindowModifiers, app::WindowModifiers) {
        (
            WindowModifiers {
                shift: true,
                control: false,
                option: true,
                command: false,
                caps_lock: true,
            },
            app::WindowModifiers {
                shift: true,
                control: false,
                option: true,
                command: false,
                caps_lock: true,
            },
        )
    }

    #[test]
    fn each_modifier_key_keeps_its_own_place() {
        let none = WindowModifiers::default();
        let nothing = app::WindowModifiers::default();
        assert_eq!(app::WindowModifiers::from(none), nothing);
        for (keys, wanted) in [
            (
                WindowModifiers {
                    shift: true,
                    ..none
                },
                app::WindowModifiers {
                    shift: true,
                    ..nothing
                },
            ),
            (
                WindowModifiers {
                    control: true,
                    ..none
                },
                app::WindowModifiers {
                    control: true,
                    ..nothing
                },
            ),
            (
                WindowModifiers {
                    option: true,
                    ..none
                },
                app::WindowModifiers {
                    option: true,
                    ..nothing
                },
            ),
            (
                WindowModifiers {
                    command: true,
                    ..none
                },
                app::WindowModifiers {
                    command: true,
                    ..nothing
                },
            ),
            (
                WindowModifiers {
                    caps_lock: true,
                    ..none
                },
                app::WindowModifiers {
                    caps_lock: true,
                    ..nothing
                },
            ),
        ] {
            assert_eq!(app::WindowModifiers::from(keys), wanted, "{keys:?}");
        }
    }

    #[test]
    fn what_the_person_does_with_a_pointer_reaches_the_app_as_it_was_given() {
        let (keys, wanted) = keys_and_their_places();
        for (action, expected) in [
            (WindowPointer::Move, app::WindowPointer::Move),
            (
                WindowPointer::Down {
                    button: WindowButton::Left,
                },
                app::WindowPointer::Down(app::WindowButton::Left),
            ),
            (
                WindowPointer::Down {
                    button: WindowButton::Right,
                },
                app::WindowPointer::Down(app::WindowButton::Right),
            ),
            (
                WindowPointer::Up {
                    button: WindowButton::Middle,
                },
                app::WindowPointer::Up(app::WindowButton::Middle),
            ),
            (
                WindowPointer::Up {
                    button: WindowButton::Left,
                },
                app::WindowPointer::Up(app::WindowButton::Left),
            ),
            (
                WindowPointer::Scroll { dx: 1.5, dy: -2.0 },
                app::WindowPointer::Scroll { dx: 1.5, dy: -2.0 },
            ),
        ] {
            let converted = app::WindowInput::from(WindowInput::Pointer {
                x: 0.25,
                y: 0.75,
                modifiers: keys,
                action,
            });
            assert_eq!(
                converted,
                app::WindowInput::Pointer {
                    x: 0.25,
                    y: 0.75,
                    modifiers: wanted,
                    action: expected,
                },
                "{action:?}"
            );
        }
    }

    #[test]
    fn what_the_person_does_with_a_key_reaches_the_app_as_it_was_given() {
        let (keys, wanted) = keys_and_their_places();
        for (code, down, repeat, text) in [
            (4, true, false, Some("a".to_owned())),
            (4, false, true, None),
            (0, true, false, Some("h\u{e9}llo".to_owned())),
        ] {
            let converted = app::WindowInput::from(WindowInput::Key {
                code,
                down,
                modifiers: keys,
                repeat,
                text: text.clone(),
            });
            assert_eq!(
                converted,
                app::WindowInput::Key {
                    code,
                    down,
                    modifiers: wanted,
                    repeat,
                    text,
                }
            );
        }
    }

    #[test]
    fn what_was_typed_is_not_printed_on_the_rust_side() {
        let key = WindowInput::Key {
            code: 4,
            down: true,
            modifiers: WindowModifiers::default(),
            repeat: false,
            text: Some("hunter2".to_owned()),
        };
        let shown = format!("{key:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(!shown.contains("code"), "{shown}");
        // Only how long it was.
        assert!(shown.contains("text_bytes: Some(7)"), "{shown}");
    }

    #[test]
    fn every_reason_the_app_gives_for_dropped_events_is_carried_over() {
        for (reason, expected) in [
            (
                app::WindowNoticeReason::ControlOff,
                WindowNoticeReason::ControlOff,
            ),
            (app::WindowNoticeReason::Paused, WindowNoticeReason::Paused),
            (app::WindowNoticeReason::Locked, WindowNoticeReason::Locked),
            (
                app::WindowNoticeReason::NotOnScreen,
                WindowNoticeReason::NotOnScreen,
            ),
            (
                app::WindowNoticeReason::TooFast,
                WindowNoticeReason::TooFast,
            ),
            (
                app::WindowNoticeReason::OutOfDate,
                WindowNoticeReason::OutOfDate,
            ),
            (
                app::WindowNoticeReason::PictureUnknown,
                WindowNoticeReason::PictureUnknown,
            ),
            (
                app::WindowNoticeReason::Resized,
                WindowNoticeReason::Resized,
            ),
            (
                app::WindowNoticeReason::OutsideWindow,
                WindowNoticeReason::OutsideWindow,
            ),
            (
                app::WindowNoticeReason::Covered,
                WindowNoticeReason::Covered,
            ),
            (
                app::WindowNoticeReason::NoKeyboard,
                WindowNoticeReason::NoKeyboard,
            ),
            (
                app::WindowNoticeReason::AppWindowInFront,
                WindowNoticeReason::AppWindowInFront,
            ),
            (
                app::WindowNoticeReason::TooManyKeys,
                WindowNoticeReason::TooManyKeys,
            ),
            (
                app::WindowNoticeReason::NotPressed,
                WindowNoticeReason::NotPressed,
            ),
            (app::WindowNoticeReason::Other, WindowNoticeReason::Other),
        ] {
            assert_eq!(WindowNoticeReason::from(reason), expected, "{reason:?}");
        }
    }

    #[test]
    fn a_notice_of_dropped_events_keeps_its_window_its_reason_and_its_count() {
        let notice = Event::from(app::Event::WindowNotice {
            id: 4,
            reason: app::WindowNoticeReason::Covered,
            count: 3,
        });
        assert_eq!(
            notice,
            Event::WindowNotice {
                id: 4,
                reason: WindowNoticeReason::Covered,
                count: 3
            }
        );
        // A number that Dart cannot hold stays a number it can.
        assert!(matches!(
            Event::from(app::Event::WindowNotice {
                id: u64::MAX,
                reason: app::WindowNoticeReason::Other,
                count: 1,
            }),
            Event::WindowNotice { id: i64::MAX, .. }
        ));
    }

    #[test]
    fn every_state_the_app_gives_for_the_sound_of_a_screen_is_carried_over() {
        for (sound, expected) in [
            (app::WindowSound::Playing, WindowSound::Playing),
            (app::WindowSound::Off, WindowSound::Off),
            (app::WindowSound::Window, WindowSound::Window),
            (app::WindowSound::NoPermission, WindowSound::NoPermission),
            (app::WindowSound::Failed, WindowSound::Failed),
            (app::WindowSound::Other, WindowSound::Other),
            (app::WindowSound::CannotPlay, WindowSound::CannotPlay),
            (app::WindowSound::NotAllowed, WindowSound::NotAllowed),
        ] {
            assert_eq!(WindowSound::from(sound), expected, "{sound:?}");
        }
    }

    #[test]
    fn the_sound_of_a_screen_keeps_its_window_and_how_it_stands() {
        assert_eq!(
            Event::from(app::Event::WindowSound {
                id: 4,
                sound: app::WindowSound::NoPermission,
            }),
            Event::WindowSound {
                id: 4,
                sound: WindowSound::NoPermission
            }
        );
        // A number that Dart cannot hold stays a number it can.
        assert!(matches!(
            Event::from(app::Event::WindowSound {
                id: u64::MAX,
                sound: app::WindowSound::Playing,
            }),
            Event::WindowSound { id: i64::MAX, .. }
        ));
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
    fn how_a_transfer_ended_is_copied_as_it_is() {
        let end = app::TransferEnd {
            request: 3,
            device_id: "cd".repeat(32),
            device_name: "Mac".to_owned(),
            files: 2,
            bytes: 99,
            preview: vec![
                app::PreviewName {
                    name: "Holiday".to_owned(),
                    folder: true,
                    files: 1,
                    bytes: 90,
                },
                app::PreviewName {
                    name: "notes.txt".to_owned(),
                    folder: false,
                    files: 0,
                    bytes: 9,
                },
            ],
            saved: vec![
                std::path::PathBuf::from("/Users/someone/Downloads/Holiday"),
                std::path::PathBuf::from("/Users/someone/Downloads/notes 2.txt"),
            ],
            result: app::TransferResult::Received { failed: 1 },
        };
        let converted = TransferEnd::from(end);
        assert_eq!(
            (converted.request, converted.files, converted.bytes),
            (3, 2, 99)
        );
        assert_eq!(converted.result, TransferResult::Received { failed: 1 });
        // What was offered is copied as the request showed it, in the same order.
        assert_eq!(
            converted.preview,
            [
                PreviewName {
                    name: "Holiday".to_owned(),
                    folder: true,
                    files: 1,
                    bytes: 90,
                },
                PreviewName {
                    name: "notes.txt".to_owned(),
                    folder: false,
                    files: 0,
                    bytes: 9,
                },
            ]
        );
        // The places come in the order they were given, as the text of each full path.
        assert_eq!(
            converted.saved,
            [
                "/Users/someone/Downloads/Holiday",
                "/Users/someone/Downloads/notes 2.txt"
            ]
        );
        // They and the names offered are shown to the person and are not for a log.
        let logged = format!("{converted:?}");
        assert!(!logged.contains("someone"), "{logged}");
        assert!(
            !logged.contains("Holiday") && !logged.contains("notes"),
            "{logged}"
        );
        assert!(logged.contains("saved: 2"), "{logged}");
        assert_eq!(
            TransferResult::from(app::TransferResult::Dropped),
            TransferResult::Dropped
        );
    }

    #[test]
    fn how_far_a_send_got_and_whether_files_are_taken_are_copied_as_they_are() {
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
