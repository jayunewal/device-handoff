// SPDX-License-Identifier: Apache-2.0

//! The windows of the app: the offers that arrive, the sessions that run, and the offers that
//! this device makes. [`Windows`](crate::Windows) says what the platform provides, and this is
//! what the app does with it.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use baton_core::{
    PairedDevice,
    streaming::{
        DecodeError, Decoder, DecoderState, End, ShareOutcome, SourceHandle, Takes, ViewHandle,
        Viewing, WindowHandler, WindowOffer, WindowShare, WindowVerdict,
    },
    util::BoxFuture,
};
use baton_proto::{DeviceId, message::DeclineReason};
use baton_stream::{
    control::{SourceMessage, Thermal},
    header::FrameHeader,
};
use tokio::sync::broadcast;

use crate::{
    Error, ErrorKind, Event, Result,
    app::{Questions, locked},
    model::{WindowAbilities, WindowEnd, WindowRequest, WindowRole, WindowSession, WindowShared},
    windows::{PickedWindow, ScreenLink, ShowLimits, WindowSharer, WindowViewer},
};

/// A session that runs, and how to end it.
#[derive(Clone)]
enum Live {
    /// A window of another device that is shown here.
    Viewing(Arc<ViewHandle>),
    /// A window of this device that another one shows.
    Sharing(Arc<SourceHandle>),
}

/// A window that the person accepted, between the answer to the offer and the start of its
/// session. A viewer shows one window at a time, so there is at most one.
struct Accepted {
    request: WindowRequest,
    screen: i64,
    /// Reaches the session of the screen, once the session runs.
    link: ScreenLink,
}

/// What the app keeps for window streaming.
pub(crate) struct Sessions {
    viewer: Option<Arc<dyn WindowViewer>>,
    sharer: Option<Arc<dyn WindowSharer>>,
    questions: Arc<Questions>,
    events: broadcast::Sender<Event>,
    live: Mutex<HashMap<u64, Live>>,
    next: AtomicU64,
    accepted: Mutex<Option<Accepted>>,
}

impl Sessions {
    pub(crate) fn new(
        viewer: Option<Arc<dyn WindowViewer>>,
        sharer: Option<Arc<dyn WindowSharer>>,
        questions: Arc<Questions>,
        events: broadcast::Sender<Event>,
    ) -> Arc<Self> {
        Arc::new(Self {
            viewer,
            sharer,
            questions,
            events,
            live: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            accepted: Mutex::new(None),
        })
    }

    pub(crate) fn abilities(&self) -> WindowAbilities {
        WindowAbilities {
            can_show: self.viewer.is_some(),
            can_share: self.sharer.is_some(),
            can_control: self.sharer.as_ref().is_some_and(|sharer| sharer.control()),
        }
    }

    /// Checks what the viewer says it can show against what the protocol allows, so that a
    /// platform that is set up wrongly fails when the app starts and not at the first window.
    pub(crate) fn check_limits(&self) -> Result<()> {
        let Some(viewer) = &self.viewer else {
            return Ok(());
        };
        let ShowLimits {
            max_width,
            max_height,
            max_fps,
            codecs,
        } = viewer.limits();
        Viewing::new(Unused, max_width, max_height, max_fps, codecs)
            .map(|_| ())
            .map_err(|error| {
                Error::new(
                    ErrorKind::Other,
                    format!("This device is set up wrongly to show windows ({error})."),
                )
            })
    }

    pub(crate) fn sharer(&self) -> Option<&Arc<dyn WindowSharer>> {
        self.sharer.as_ref()
    }

    /// Ends the session `id`, if it runs. Returns whether it did.
    pub(crate) fn end(&self, id: u64) -> bool {
        let live = locked(&self.live).get(&id).cloned();
        match live {
            Some(Live::Viewing(handle)) => handle.end(),
            Some(Live::Sharing(handle)) => handle.end(),
            None => return false,
        }
        true
    }

    fn emit(&self, event: Event) {
        // Nobody listening is fine.
        let _ = self.events.send(event);
    }

    /// Takes note of a session that started, tells the screens, and reports its end when it comes.
    fn begin(self: &Arc<Self>, session: WindowSession, live: Live) {
        let id = session.id;
        locked(&self.live).insert(id, live.clone());
        self.emit(Event::WindowStarted(session));
        let sessions = Arc::clone(self);
        tokio::spawn(async move {
            let end = match &live {
                Live::Viewing(handle) => handle.ended().await,
                Live::Sharing(handle) => handle.ended().await,
            };
            locked(&sessions.live).remove(&id);
            tracing::debug!(session = id, ?end, "window session ended");
            sessions.emit(Event::WindowEnded {
                id,
                end: window_end(end),
            });
        });
    }

    fn next_id(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    /// The person is asked whether to show the window, and if they say yes the platform makes the
    /// screen for it.
    async fn offered(&self, from: &PairedDevice, offer: &WindowOffer) -> WindowVerdict {
        let Some(viewer) = &self.viewer else {
            return WindowVerdict::Refuse;
        };
        let device_id = from.id.to_string();
        let device_name = from.display_name().to_owned();
        let (accepted, id) = self
            .questions
            .ask_window(|id| WindowRequest {
                id,
                device_id: device_id.clone(),
                device_name: device_name.clone(),
                title: offer.title.clone(),
                app: offer.app.clone(),
                width: offer.width,
                height: offer.height,
                control: offer.control,
            })
            .await;
        if !accepted {
            return WindowVerdict::Refuse;
        }
        let failed = |message: &str| {
            self.emit(Event::WindowRequestEnded {
                id,
                failure: Some(Error::new(ErrorKind::Other, message).message().to_owned()),
            });
            WindowVerdict::Refuse
        };
        let link = ScreenLink::default();
        let screen = match viewer.open(offer, link.clone()) {
            Ok(screen) => screen,
            Err(error) => {
                tracing::warn!(error = %error, "the platform could not show a window");
                return failed(error.message());
            }
        };
        let limits = viewer.limits();
        let viewing = match Viewing::new(
            screen.decoder,
            limits.max_width,
            limits.max_height,
            limits.max_fps,
            limits.codecs,
        ) {
            Ok(viewing) => viewing,
            Err(error) => {
                tracing::error!(error = %error, "the platform's limits are not within the protocol");
                return failed("This device cannot show windows.");
            }
        };
        *locked(&self.accepted) = Some(Accepted {
            request: WindowRequest {
                id,
                device_id,
                device_name,
                title: offer.title.clone(),
                app: offer.app.clone(),
                width: offer.width,
                height: offer.height,
                control: offer.control,
            },
            screen: screen.id,
            link,
        });
        WindowVerdict::Accept(viewing)
    }

    /// The session of a window that was accepted runs. The node asks about one window at a time
    /// and starts it before it asks about the next, so what starts is what the person accepted.
    fn started(self: &Arc<Self>, handle: ViewHandle) {
        let Some(accepted) = locked(&self.accepted).take() else {
            // A window nobody accepted: the handle's drop ends it.
            tracing::warn!("a window started that the person had not accepted");
            return;
        };
        let WindowRequest {
            device_id,
            device_name,
            title,
            app,
            width,
            height,
            control,
            ..
        } = accepted.request;
        let session = WindowSession {
            id: self.next_id(),
            role: WindowRole::Viewing,
            device_id,
            device_name,
            title,
            app,
            width,
            height,
            control,
            screen: Some(accepted.screen),
        };
        let handle = Arc::new(handle);
        accepted.link.connect(&handle);
        self.begin(session, Live::Viewing(handle));
    }

    /// Offers the window that the person picked to `peer`, and starts the session if it says yes.
    pub(crate) async fn share(
        self: &Arc<Self>,
        node: &baton_core::Node,
        peer: &DeviceId,
        name: &str,
        mut picked: Box<dyn PickedWindow>,
        control: bool,
    ) -> Result<WindowShared> {
        let (width, height) = picked.size();
        let (title, app) = (picked.title(), picked.app());
        let mut window = WindowShare::new(title.clone(), app.clone(), width, height);
        if control {
            window = window.with_control();
        }
        let events = picked.events();
        // The platform's part is used twice, once to choose the picture and once to start the
        // capture, and the offer in between takes the first of these as a closure.
        let picked = Arc::new(Mutex::new(Some(picked)));
        let chooser = Arc::clone(&picked);
        let choose = move |takes: &Takes| {
            locked(&chooser)
                .as_mut()
                .and_then(|picked| picked.prepare(takes))
        };
        match node.share(peer, window, events, choose).await {
            Ok(ShareOutcome::Started(handle)) => {
                let Some(picked) = locked(&picked).take() else {
                    return Err(Error::new(ErrorKind::Other, "The window was lost."));
                };
                let handle = Arc::new(handle);
                let session = WindowSession {
                    id: self.next_id(),
                    role: WindowRole::Sharing,
                    device_id: peer.to_string(),
                    device_name: name.to_owned(),
                    title: non_empty(&title),
                    app: non_empty(&app),
                    width,
                    height,
                    control,
                    screen: None,
                };
                self.begin(session.clone(), Live::Sharing(Arc::clone(&handle)));
                picked.start(handle);
                Ok(WindowShared::Started(session))
            }
            Ok(ShareOutcome::Declined(reason)) => {
                Ok(WindowShared::Declined(declined_window(name, reason)))
            }
            Ok(ShareOutcome::Unmatched) => Ok(WindowShared::Declined(format!(
                "{name} cannot show the picture that this device makes of the window."
            ))),
            Ok(_) => Ok(WindowShared::Declined(format!(
                "{name} did not take the window."
            ))),
            Err(error) => Err(Error::sharing(&error, name)),
        }
    }
}

/// A decoder that is never used: it stands in for the real one when the limits are checked.
struct Unused;

impl Decoder for Unused {
    fn decode(&mut self, _: &FrameHeader, _: Vec<u8>) -> std::result::Result<(), DecodeError> {
        Err(DecodeError)
    }

    fn show(&mut self, _: u64) {}

    fn source(&mut self, _: SourceMessage) {}

    fn state(&self) -> DecoderState {
        DecoderState {
            decode_us: 0,
            thermal: Thermal::Nominal,
        }
    }

    fn ended(&mut self) {}
}

/// The text of a title or a name, or `None` if there is none.
fn non_empty(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// How a session ended, as a screen is told.
fn window_end(end: End) -> WindowEnd {
    match end {
        End::Local => WindowEnd::ThisDevice,
        End::Peer => WindowEnd::OtherDevice,
        End::Connection | End::Silent | End::Stalled => WindowEnd::Lost,
        End::NoControl => WindowEnd::Failed(
            "The other device did not start sending the window in time.".to_owned(),
        ),
        End::Violation => WindowEnd::Failed(
            "The other device sent something that this version of Baton does not understand."
                .to_owned(),
        ),
        _ => WindowEnd::Failed("Something went wrong inside Baton.".to_owned()),
    }
}

/// What to tell the person when the other device did not take the window.
fn declined_window(device: &str, reason: DeclineReason) -> String {
    match reason {
        DeclineReason::Refused => format!("{device} did not accept the window."),
        DeclineReason::Blocked => {
            format!(
                "{device} does not take windows from this device: its settings do not allow it."
            )
        }
        DeclineReason::Unsupported => {
            format!("{device} cannot show windows. It may need a newer version of Baton.")
        }
        DeclineReason::RateLimited => {
            format!("{device} is busy with another window. Try again in a moment.")
        }
        DeclineReason::Invalid => format!("{device} could not read the offer of the window."),
        _ => format!("{device} did not take the window."),
    }
}

/// The handler that the node asks about windows.
pub(crate) struct Handler(pub(crate) Arc<Sessions>);

impl WindowHandler for Handler {
    fn offered<'a>(
        &'a self,
        from: &'a PairedDevice,
        offer: &'a WindowOffer,
    ) -> BoxFuture<'a, WindowVerdict> {
        Box::pin(self.0.offered(from, offer))
    }

    fn started(&self, _from: &PairedDevice, handle: ViewHandle) {
        self.0.started(handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_end_is_told_as_a_screen_needs_it() {
        assert_eq!(window_end(End::Local), WindowEnd::ThisDevice);
        assert_eq!(window_end(End::Peer), WindowEnd::OtherDevice);
        for lost in [End::Connection, End::Silent, End::Stalled] {
            assert_eq!(window_end(lost), WindowEnd::Lost, "{lost:?}");
        }
        for failed in [End::NoControl, End::Violation, End::Internal] {
            assert!(
                matches!(window_end(failed), WindowEnd::Failed(_)),
                "{failed:?}"
            );
        }
    }

    #[test]
    fn a_title_that_is_blank_is_no_title() {
        assert_eq!(non_empty("  "), None);
        assert_eq!(non_empty(""), None);
        assert_eq!(non_empty(" Report "), Some("Report".to_owned()));
    }

    #[test]
    fn what_the_person_is_told_of_a_window_that_was_not_taken_names_the_device() {
        for reason in [
            DeclineReason::Refused,
            DeclineReason::Blocked,
            DeclineReason::Unsupported,
            DeclineReason::RateLimited,
            DeclineReason::Invalid,
            DeclineReason::Other,
        ] {
            assert!(declined_window("Kitchen iPad", reason).contains("Kitchen iPad"));
        }
    }
}
