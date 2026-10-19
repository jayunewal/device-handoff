// SPDX-License-Identifier: Apache-2.0

//! The windows of the app: the offers that arrive, the sessions that run, and the offers that
//! this device makes. [`Windows`](crate::Windows) says what the platform provides, and this is
//! what the app does with it.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
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
use baton_stream::{control::SourceMessage, header::FrameHeader};
use tokio::sync::broadcast;

use crate::{
    Error, ErrorKind, Event, Result,
    app::{Questions, locked},
    model::{WindowAbilities, WindowEnd, WindowRequest, WindowRole, WindowSession, WindowShared},
    windows::{PickedWindow, ScreenLink, SharingFailure, WindowSharer, WindowViewer},
};

/// A session that runs, and how to end it.
#[derive(Clone)]
enum Live {
    /// A window of another device that is shown here.
    Viewing(Arc<ViewHandle>),
    /// A window of this device that another one shows. The platform has the handle, and ends the
    /// session by letting go of it, so the app keeps no copy that would keep it going.
    Sharing(Weak<SourceHandle>),
}

/// A window that the person accepted, between the answer to the offer and the start of its
/// session. A viewer shows one window at a time, so there is at most one.
struct Accepted {
    request: WindowRequest,
    screen: i64,
    /// Reaches the session of the screen, once the session runs.
    link: ScreenLink,
    /// Tells the screens how large the picture is, once they know the window.
    pictures: Arc<Pictures>,
}

/// What the source of a window that is shown here says about the size of its picture, kept until
/// the screens know the window.
///
/// The source says the size before its first picture, and the node tells the app that the
/// session runs at about the same moment, on tasks of their own, so the size can come first. It
/// waits here for the window and is told right after [`Event::WindowStarted`], so that a screen
/// always hears of a window before it hears of its picture.
struct Pictures {
    events: broadcast::Sender<Event>,
    state: Mutex<PictureState>,
}

#[derive(Default)]
struct PictureState {
    /// The window, once the screens have been told of it.
    session: Option<u64>,
    /// The size that the source said last.
    size: Option<(u32, u32)>,
}

impl Pictures {
    fn new(events: broadcast::Sender<Event>) -> Arc<Self> {
        Arc::new(Self {
            events,
            state: Mutex::new(PictureState::default()),
        })
    }

    /// The source says that its pictures are this size from now on.
    fn said(&self, width: u32, height: u32) {
        let mut state = locked(&self.state);
        state.size = Some((width, height));
        if let Some(id) = state.session {
            // Told under the lock, so that it cannot overtake the size that `began` tells.
            self.tell(id, width, height);
        }
    }

    /// The screens have been told of the window `id`: whatever the source said before is told now.
    fn began(&self, id: u64) {
        let mut state = locked(&self.state);
        state.session = Some(id);
        if let Some((width, height)) = state.size {
            self.tell(id, width, height);
        }
    }

    fn tell(&self, id: u64, width: u32, height: u32) {
        // Nobody listening is fine.
        let _ = self.events.send(Event::WindowPicture { id, width, height });
    }
}

/// The decoder of the platform, with the app hearing what the source says about its picture. It
/// passes everything on as it is.
struct Watching {
    decoder: Box<dyn Decoder>,
    pictures: Arc<Pictures>,
}

impl Decoder for Watching {
    fn decode(
        &mut self,
        frame: &FrameHeader,
        picture: Vec<u8>,
    ) -> std::result::Result<(), DecodeError> {
        self.decoder.decode(frame, picture)
    }

    fn show(&mut self, number: u64) {
        self.decoder.show(number);
    }

    fn source(&mut self, message: SourceMessage) {
        if let SourceMessage::Config(config) = &message {
            self.pictures.said(config.width, config.height);
        }
        self.decoder.source(message);
    }

    fn state(&self) -> DecoderState {
        self.decoder.state()
    }

    fn ended(&mut self) {
        self.decoder.ended();
    }
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
        viewer.limits().check().map_err(|error| {
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
            Some(Live::Sharing(handle)) => match handle.upgrade() {
                Some(handle) => handle.end(),
                // The platform let go of the handle, which ended the session. Its entry goes when
                // the task that waits for the end gets to it.
                None => return false,
            },
            None => return false,
        }
        true
    }

    fn emit(&self, event: Event) {
        // Nobody listening is fine.
        let _ = self.events.send(event);
    }

    /// Takes note of a session that started, tells the screens, and reports its end when it comes.
    /// `ended` waits for the end and says how it is told. It is given apart from `live` because a
    /// window that this device shares is waited for without holding its handle, which the
    /// platform owns. `pictures` is given for a window that is shown here: what its source said of
    /// the size of the picture is told right after the window, and never before it.
    fn begin(
        self: &Arc<Self>,
        session: WindowSession,
        live: Live,
        ended: BoxFuture<'static, WindowEnd>,
        pictures: Option<&Pictures>,
    ) {
        let id = session.id;
        locked(&self.live).insert(id, live);
        self.emit(Event::WindowStarted(session));
        if let Some(pictures) = pictures {
            pictures.began(id);
        }
        let sessions = Arc::clone(self);
        tokio::spawn(async move {
            let end = ended.await;
            locked(&sessions.live).remove(&id);
            tracing::debug!(session = id, ?end, "window session ended");
            sessions.emit(Event::WindowEnded { id, end });
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
        let pictures = Pictures::new(self.events.clone());
        let viewing = match Viewing::new(
            Watching {
                decoder: screen.decoder,
                pictures: Arc::clone(&pictures),
            },
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
            pictures,
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
        let ended = {
            let handle = Arc::clone(&handle);
            Box::pin(async move { window_end(handle.ended().await) })
        };
        self.begin(
            session,
            Live::Viewing(handle),
            ended,
            Some(&accepted.pictures),
        );
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
                let failure = SharingFailure::new(&handle);
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
                self.begin(
                    session.clone(),
                    Live::Sharing(Arc::downgrade(&handle)),
                    Box::pin(sharing_end(handle.on_end(), failure.clone())),
                    None,
                );
                picked.start(handle, failure);
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

/// The text of a title or a name, or `None` if there is none.
fn non_empty(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// How a window that this device shares ended, as a screen is told, once its session is over.
async fn sharing_end(ended: impl Future<Output = End>, failure: SharingFailure) -> WindowEnd {
    sharing_outcome(ended.await, failure.message())
}

/// How a window that this device shares ended, as a screen is told: with the sentence that the
/// platform gave if it ended the session to say that something went wrong, made fit for a screen,
/// and as any other session otherwise. A session that the other device or the connection ended
/// first is told as that, whatever the platform says afterwards.
fn sharing_outcome(end: End, sentence: Option<String>) -> WindowEnd {
    if end == End::Local {
        let sentence =
            sentence.map(|message| Error::new(ErrorKind::Other, message).message().to_owned());
        if let Some(sentence) = sentence.filter(|sentence| !sentence.is_empty()) {
            return WindowEnd::Failed(sentence);
        }
    }
    window_end(end)
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
    use baton_proto::{id::SessionId, stream::Codec};
    use baton_stream::{
        control::{Config, Pause, PauseReason, Thermal},
        header::Flags,
    };
    use tokio::sync::broadcast::error::TryRecvError;

    use super::*;

    fn picture(id: u64, width: u32, height: u32) -> Event {
        Event::WindowPicture { id, width, height }
    }

    fn config(width: u32, height: u32) -> SourceMessage {
        SourceMessage::Config(Config {
            epoch: 0,
            codec: Codec::H264,
            width,
            height,
            fps: 30,
        })
    }

    fn pictures() -> (Arc<Pictures>, broadcast::Receiver<Event>) {
        let (events, heard) = broadcast::channel(8);
        (Pictures::new(events), heard)
    }

    #[test]
    fn a_size_that_comes_when_the_window_is_known_is_told_at_once() {
        let (pictures, mut heard) = pictures();
        pictures.began(3);
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
        pictures.said(1280, 800);
        assert_eq!(heard.try_recv(), Ok(picture(3, 1280, 800)));
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn a_size_that_comes_before_the_window_is_known_is_told_when_it_is() {
        let (pictures, mut heard) = pictures();
        pictures.said(1280, 800);
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
        pictures.began(3);
        assert_eq!(heard.try_recv(), Ok(picture(3, 1280, 800)));
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn of_the_sizes_that_came_before_the_window_is_known_the_last_is_told() {
        let (pictures, mut heard) = pictures();
        pictures.said(1280, 800);
        pictures.said(640, 400);
        pictures.began(3);
        assert_eq!(heard.try_recv(), Ok(picture(3, 640, 400)));
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn every_new_size_is_told_in_the_order_it_came() {
        let (pictures, mut heard) = pictures();
        pictures.began(3);
        pictures.said(1280, 800);
        pictures.said(800, 800);
        pictures.said(1280, 800);
        for expected in [(1280, 800), (800, 800), (1280, 800)] {
            assert_eq!(heard.try_recv(), Ok(picture(3, expected.0, expected.1)));
        }
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    fn window(id: u64, role: WindowRole) -> WindowSession {
        WindowSession {
            id,
            role,
            device_id: "ab".repeat(32),
            device_name: "Studio Mac".to_owned(),
            title: None,
            app: None,
            width: 1440,
            height: 900,
            control: false,
            screen: Some(100),
        }
    }

    #[tokio::test]
    async fn the_screens_hear_of_a_window_before_they_hear_of_its_picture() {
        let (events, mut heard) = broadcast::channel(8);
        let sessions = Sessions::new(None, None, Questions::new(events.clone()), events.clone());
        let pictures = Pictures::new(events);
        // The source has said the size already when the app makes the window known.
        pictures.said(1280, 800);
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
        let shown = window(6, WindowRole::Viewing);

        // What the entry holds does not matter here, so it is the one that needs no handle.
        sessions.begin(
            shown.clone(),
            Live::Sharing(Weak::new()),
            Box::pin(std::future::pending()),
            Some(&pictures),
        );

        assert_eq!(heard.try_recv(), Ok(Event::WindowStarted(shown)));
        assert_eq!(heard.try_recv(), Ok(picture(6, 1280, 800)));
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    #[tokio::test]
    async fn a_window_that_has_no_pictures_to_wait_for_is_told_alone() {
        let (events, mut heard) = broadcast::channel(8);
        let sessions = Sessions::new(None, None, Questions::new(events.clone()), events);
        let shared = window(2, WindowRole::Sharing);

        sessions.begin(
            shared.clone(),
            Live::Sharing(Weak::new()),
            Box::pin(std::future::pending()),
            None,
        );

        assert_eq!(heard.try_recv(), Ok(Event::WindowStarted(shared)));
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    /// A decoder that writes down what it is given.
    struct Recorder(Arc<Mutex<Vec<String>>>);

    impl Decoder for Recorder {
        fn decode(
            &mut self,
            frame: &FrameHeader,
            picture: Vec<u8>,
        ) -> std::result::Result<(), DecodeError> {
            locked(&self.0).push(format!(
                "decode {} of {} bytes",
                frame.number,
                picture.len()
            ));
            Ok(())
        }

        fn show(&mut self, number: u64) {
            locked(&self.0).push(format!("show {number}"));
        }

        fn source(&mut self, message: SourceMessage) {
            locked(&self.0).push(format!("source {message:?}"));
        }

        fn state(&self) -> DecoderState {
            DecoderState {
                decode_us: 1234,
                thermal: Thermal::Fair,
            }
        }

        fn ended(&mut self) {
            locked(&self.0).push("ended".to_owned());
        }
    }

    fn watching() -> (
        Watching,
        Arc<Mutex<Vec<String>>>,
        broadcast::Receiver<Event>,
    ) {
        let (pictures, heard) = pictures();
        pictures.began(5);
        let written = Arc::new(Mutex::new(Vec::new()));
        let decoder = Watching {
            decoder: Box::new(Recorder(Arc::clone(&written))),
            pictures,
        };
        (decoder, written, heard)
    }

    #[test]
    fn the_decoder_of_the_platform_is_given_everything_as_it_comes() {
        let (mut decoder, written, _) = watching();
        let header = FrameHeader {
            session: SessionId::from_bytes([7; SessionId::LEN]),
            number: 9,
            depends_on: 9,
            chain: 0,
            epoch: 0,
            capture_time: 0,
            flags: Flags::KEY,
        };
        assert!(decoder.decode(&header, vec![0; 12]).is_ok());
        decoder.show(9);
        decoder.source(SourceMessage::Resume);
        decoder.ended();
        assert_eq!(
            *locked(&written),
            ["decode 9 of 12 bytes", "show 9", "source Resume", "ended"]
        );
        let state = decoder.state();
        assert_eq!((state.decode_us, state.thermal), (1234, Thermal::Fair));
    }

    #[test]
    fn what_the_source_says_is_given_on_and_only_its_picture_is_told_to_the_screens() {
        let (mut decoder, written, mut heard) = watching();
        decoder.source(config(1000, 500));
        assert_eq!(heard.try_recv(), Ok(picture(5, 1000, 500)));
        for message in [
            SourceMessage::Pause(Pause {
                reason: PauseReason::Hidden,
            }),
            SourceMessage::Resume,
        ] {
            decoder.source(message);
        }
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
        let written = locked(&written);
        assert_eq!(written.len(), 3);
        assert!(written[0].starts_with("source Config("), "{written:?}");
        assert!(written[1].starts_with("source Pause("), "{written:?}");
        assert_eq!(written[2], "source Resume");
    }

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
    fn a_window_that_the_platform_ended_is_told_with_its_sentence_if_it_gave_one() {
        assert_eq!(
            sharing_outcome(End::Local, Some("The capture stopped.".to_owned())),
            WindowEnd::Failed("The capture stopped.".to_owned())
        );
        // The sentence is made fit for a screen.
        assert_eq!(
            sharing_outcome(
                End::Local,
                Some("The capture\nstopped.\u{202e}\u{7}".to_owned())
            ),
            WindowEnd::Failed("The capture stopped.".to_owned())
        );
        // No sentence, and a sentence with nothing in it, are the person closing the window.
        for nothing in [
            None,
            Some(String::new()),
            Some(" \n\t".to_owned()),
            Some("\u{200b}".to_owned()),
        ] {
            assert_eq!(
                sharing_outcome(End::Local, nothing.clone()),
                WindowEnd::ThisDevice,
                "{nothing:?}"
            );
        }
    }

    #[test]
    fn a_window_that_something_else_ended_is_told_as_that_whatever_the_platform_says() {
        for end in [
            End::Peer,
            End::Connection,
            End::Silent,
            End::Stalled,
            End::NoControl,
            End::Violation,
            End::Internal,
        ] {
            for sentence in [None, Some("The capture stopped.".to_owned())] {
                assert_eq!(
                    sharing_outcome(end, sentence.clone()),
                    window_end(end),
                    "{end:?}: {sentence:?}"
                );
            }
        }
    }

    #[test]
    fn ending_a_window_that_the_platform_let_go_of_does_nothing() {
        let (events, _) = broadcast::channel(8);
        let sessions = Sessions::new(None, None, Questions::new(events.clone()), events);
        // The entry of a session whose handle the platform dropped, before the task that waits
        // for the end has removed it: the drop ended the session, and nothing is left to end.
        locked(&sessions.live).insert(7, Live::Sharing(Weak::new()));
        assert!(!sessions.end(7));
        assert!(!sessions.end(8));
    }

    #[test]
    fn a_title_that_is_blank_is_no_title() {
        assert_eq!(non_empty("  "), None);
        assert_eq!(non_empty(""), None);
        assert_eq!(non_empty(" Report "), Some("Report".to_owned()));
    }

    #[test]
    fn what_the_person_is_told_of_a_window_that_was_not_taken_names_the_device_and_the_reason() {
        for (reason, why) in [
            (DeclineReason::Refused, "did not accept the window"),
            (DeclineReason::Blocked, "its settings do not allow it"),
            (DeclineReason::Unsupported, "may need a newer version"),
            (DeclineReason::RateLimited, "busy with another window"),
            (DeclineReason::Invalid, "could not read the offer"),
            (DeclineReason::Other, "did not take the window"),
        ] {
            let told = declined_window("Kitchen iPad", reason);
            assert!(told.contains("Kitchen iPad"), "{reason:?}: {told}");
            assert!(told.contains(why), "{reason:?}: {told}");
        }
    }
}
