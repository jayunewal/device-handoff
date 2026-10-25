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
        DecodeError, Decoder, DecoderState, End, ShareOutcome, SourceHandle, StartOutcome,
        StartSlot, Takes, ViewHandle, Viewing, Wanted, WindowHandler, WindowOffer, WindowShare,
        WindowVerdict,
    },
    util::BoxFuture,
};
use baton_proto::{DeviceId, message::DeclineReason};
use baton_stream::{control::SourceMessage, gate::Notice, header::FrameHeader};
use tokio::sync::broadcast;

use crate::{
    Error, ErrorKind, Event, Result,
    app::{Questions, locked},
    model::{
        ScreenStarted, WindowAbilities, WindowEnd, WindowInput, WindowRequest, WindowRole,
        WindowSession, WindowShared,
    },
    windows::{PickedWindow, ScreenLink, SharingFailure, WindowSharer, WindowViewer, noun},
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

/// What a window that is to be shown here needs before the node can be asked to take it.
struct Prepared {
    /// The number that the user interface draws the screen with.
    screen: i64,
    /// Reaches the session of the screen, once the session runs.
    link: ScreenLink,
    /// Tells the screens how large the picture is, once they know the window.
    pictures: Arc<Pictures>,
    /// What this device takes, and the decoder that shows it.
    viewing: Viewing,
}

/// What the source of a window that is shown here says that the screens need: the size of its
/// picture, kept until the screens know the window, and the events of the person that it dropped.
///
/// The source says the size before its first picture, and the node tells the app that the
/// session runs at about the same moment, on tasks of their own, so the size can come first. It
/// waits here for the window and is told right after [`Event::WindowStarted`], so that a screen
/// always hears of a window before it hears of its picture. A notice answers what the person did
/// on the window, which they can do only once they know it, so it is told when the window is
/// known and dropped when it is not.
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

    /// The source did not do some of what the person did, for the reason in the notice.
    fn noticed(&self, notice: Notice) {
        let state = locked(&self.state);
        // Under the lock, so that it cannot overtake the window or a size that `began` tells.
        if let Some(id) = state.session {
            // The sentence on the screen cannot tell every reason apart, and a person who reports
            // that nothing happened can read the log of this device. The other device sends at most
            // one notice a second, so this is a line a second at the most.
            tracing::info!(
                reason = ?notice.reason,
                count = notice.count,
                "the other device did not do some of what was done on the window"
            );
            // Nobody listening is fine.
            let _ = self.events.send(Event::WindowNotice {
                id,
                reason: notice.reason.into(),
                count: notice.count,
            });
        }
    }
}

/// The decoder of the platform, with the app hearing what the source says about its picture and
/// about the events it dropped. It passes everything on as it is.
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
        match &message {
            SourceMessage::Config(config) => self.pictures.said(config.width, config.height),
            SourceMessage::Notice(notice) => self.pictures.noticed(*notice),
            _ => {}
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
            can_give_screen: self
                .sharer
                .as_ref()
                .is_some_and(|sharer| sharer.can_give_screen()),
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

    /// The window `id`, if it is shown here and runs.
    fn viewing(&self, id: u64) -> Option<Arc<ViewHandle>> {
        match locked(&self.live).get(&id) {
            Some(Live::Viewing(handle)) => Some(Arc::clone(handle)),
            _ => None,
        }
    }

    /// Gives what the person did on the window `id`. Returns whether it was taken: a window that
    /// is not shown here, one that the other device did not let this one control, and an event that
    /// has no meaning (a touch outside the picture, a release of what is not down) take nothing.
    pub(crate) fn input(&self, id: u64, input: WindowInput) -> bool {
        self.viewing(id)
            .is_some_and(|handle| handle.input(input.into()).is_ok())
    }

    /// Lets go of every button and key that is down on the window `id`, if it is shown here.
    pub(crate) fn release_input(&self, id: u64) {
        if let Some(handle) = self.viewing(id) {
            handle.release_input();
        }
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
                display: offer.display,
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
        let Prepared {
            screen,
            link,
            pictures,
            viewing,
        } = match self.prepare(viewer, offer) {
            Ok(prepared) => prepared,
            Err(sentence) => return failed(&sentence),
        };
        *locked(&self.accepted) = Some(Accepted {
            request: WindowRequest {
                id,
                device_id,
                device_name,
                display: offer.display,
                title: offer.title.clone(),
                app: offer.app.clone(),
                width: offer.width,
                height: offer.height,
                control: offer.control,
            },
            screen,
            link,
            pictures,
        });
        WindowVerdict::Accept(viewing)
    }

    /// The platform makes the screen for `offer`, and the node is told what this device takes and
    /// shows the picture with. The sentence is what the person is told if that cannot be done.
    fn prepare(
        &self,
        viewer: &Arc<dyn WindowViewer>,
        offer: &WindowOffer,
    ) -> std::result::Result<Prepared, String> {
        let link = ScreenLink::default();
        let screen = viewer.open(offer, link.clone()).map_err(|error| {
            tracing::warn!(error = %error, "the platform could not show a window");
            error.message().to_owned()
        })?;
        let limits = viewer.limits();
        let pictures = Pictures::new(self.events.clone());
        let viewing = Viewing::new(
            Watching {
                decoder: screen.decoder,
                pictures: Arc::clone(&pictures),
            },
            limits.max_width,
            limits.max_height,
            limits.max_fps,
            limits.codecs,
        )
        .map_err(|error| {
            tracing::error!(error = %error, "the platform's limits are not within the protocol");
            "This device cannot show windows.".to_owned()
        })?;
        Ok(Prepared {
            screen: screen.id,
            link,
            pictures,
            viewing,
        })
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
            display,
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
            display,
            title,
            app,
            width,
            height,
            control,
            screen: Some(accepted.screen),
        };
        self.begin_viewing(session, handle, &accepted.link, &accepted.pictures);
    }

    /// The session `session` of a window that is shown here runs with `handle`.
    fn begin_viewing(
        self: &Arc<Self>,
        session: WindowSession,
        handle: ViewHandle,
        link: &ScreenLink,
        pictures: &Pictures,
    ) {
        let handle = Arc::new(handle);
        link.connect(&handle);
        let ended = {
            let handle = Arc::clone(&handle);
            Box::pin(async move { window_end(handle.ended().await) })
        };
        self.begin(session, Live::Viewing(handle), ended, Some(pictures));
    }

    /// Asks `peer` to start sharing its screen with this device, and shows it if it does. No
    /// person at `peer` is asked: they allowed this device beforehand, in the settings for it.
    /// `control` is whether the person here wishes to control the screen; `peer` grants no more
    /// than the settings for this device allow.
    pub(crate) async fn start(
        self: &Arc<Self>,
        node: &baton_core::Node,
        peer: &DeviceId,
        name: &str,
        control: bool,
    ) -> Result<ScreenStarted> {
        let Some(viewer) = &self.viewer else {
            return Err(Error::new(
                ErrorKind::Other,
                "This device cannot show screens.",
            ));
        };
        // The node needs the decoder to ask, and the offer that answers comes after, so the
        // platform makes the screen for a display of no known size.
        let Prepared {
            screen,
            link,
            pictures,
            viewing,
        } = self
            .prepare(viewer, &WindowOffer::asked_screen(control))
            .map_err(|sentence| Error::new(ErrorKind::Other, sentence))?;
        match node.start(peer, Wanted::Screen, control, viewing).await {
            Ok(StartOutcome::Started(handle)) => {
                let session = WindowSession {
                    id: self.next_id(),
                    role: WindowRole::Viewing,
                    device_id: peer.to_string(),
                    device_name: name.to_owned(),
                    display: true,
                    title: None,
                    app: None,
                    width: 0,
                    height: 0,
                    control: handle.control(),
                    screen: Some(screen),
                };
                self.begin_viewing(session.clone(), handle, &link, &pictures);
                Ok(ScreenStarted::Started(session))
            }
            Ok(StartOutcome::NotStarted(why)) => Ok(ScreenStarted::NotStarted(
                Error::not_started(name, why).message().to_owned(),
            )),
            // An outcome that this version does not know.
            Ok(_) => Ok(ScreenStarted::NotStarted(format!(
                "{name} did not start its screen."
            ))),
            Err(error) => Err(Error::asking(&error, name)),
        }
    }

    /// Offers the window that the person picked to `peer`, and starts the session if it says yes.
    /// `asked` is the place of the session when `peer` asked this device to start it: the offer is
    /// then made under the id that `peer` chose, and `control` is no more than the slot allows.
    pub(crate) async fn share(
        self: &Arc<Self>,
        node: &baton_core::Node,
        peer: &DeviceId,
        name: &str,
        mut picked: Box<dyn PickedWindow>,
        control: bool,
        asked: Option<StartSlot>,
    ) -> Result<WindowShared> {
        let (width, height) = picked.size();
        let (title, app) = (picked.title(), picked.app());
        let display = picked.display();
        let mut window = WindowShare::new(title.clone(), app.clone(), width, height);
        if display {
            window = window.whole_display();
        }
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
        let outcome = match asked {
            None => node.share(peer, window, events, choose).await,
            Some(slot) => node.share_asked(slot, window, events, choose).await,
        };
        match outcome {
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
                    display,
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
            Ok(ShareOutcome::Declined(reason)) => Ok(WindowShared::Declined(declined_window(
                name, reason, display,
            ))),
            Ok(ShareOutcome::Unmatched) => Ok(WindowShared::Declined(format!(
                "{name} cannot show the picture that this device makes of the {}.",
                noun(display)
            ))),
            Ok(_) => Ok(WindowShared::Declined(declined_window(
                name,
                DeclineReason::Other,
                display,
            ))),
            Err(error) => Err(Error::sharing(&error, name, display)),
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

/// What to tell the person when the other device did not take the window, or the screen.
fn declined_window(device: &str, reason: DeclineReason, display: bool) -> String {
    let what = noun(display);
    match reason {
        DeclineReason::Refused => format!("{device} did not accept the {what}."),
        DeclineReason::Blocked => {
            format!(
                "{device} does not take {what}s from this device: its settings do not allow it."
            )
        }
        DeclineReason::Unsupported => {
            format!("{device} cannot show {what}s. It may need a newer version of Baton.")
        }
        // What the other device is busy with is a window or a screen, whichever was offered.
        DeclineReason::RateLimited => {
            format!("{device} is busy with another window or screen. Try again in a moment.")
        }
        DeclineReason::Invalid => format!("{device} could not read the offer of the {what}."),
        _ => format!("{device} did not take the {what}."),
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
        gate::Reason,
        header::Flags,
    };
    use tokio::sync::broadcast::error::TryRecvError;

    use super::*;
    use crate::model::{WindowButton, WindowModifiers, WindowNoticeReason, WindowPointer};

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
        logging();
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

    fn notice(reason: Reason, count: u32) -> Notice {
        Notice { reason, count }
    }

    fn noticed(id: u64, reason: WindowNoticeReason, count: u32) -> Event {
        Event::WindowNotice { id, reason, count }
    }

    #[test]
    fn a_notice_that_comes_when_the_window_is_known_is_told_at_once() {
        let (pictures, mut heard) = pictures();
        pictures.began(3);
        pictures.noticed(notice(Reason::Covered, 4));
        assert_eq!(
            heard.try_recv(),
            Ok(noticed(3, WindowNoticeReason::Covered, 4))
        );
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    /// What the logger of the tests wrote, whichever test it was for.
    static LOGGED: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());

    /// Where the logger of the tests writes: a place that a test can read.
    #[derive(Clone, Copy)]
    struct Captured;

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            LOGGED.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl tracing_subscriber::fmt::MakeWriter<'_> for Captured {
        type Writer = Self;

        fn make_writer(&self) -> Self {
            *self
        }
    }

    /// Puts the logger of the tests in place, once for all of them, and it is in place before
    /// any test runs the line that logs (`pictures` calls this). A logger that only the thread
    /// of one test has can miss the line of that test: the first thread that runs the line with
    /// no logger of its own makes the logging library decide, for every thread, that nobody
    /// listens to that line, and the test then waits for a line that is never written.
    fn logging() {
        static IN_PLACE: std::sync::Once = std::sync::Once::new();
        IN_PLACE.call_once(|| {
            tracing::subscriber::set_global_default(
                tracing_subscriber::fmt()
                    .with_writer(Captured)
                    .with_ansi(false)
                    .with_target(false)
                    .with_thread_names(true)
                    .finish(),
            )
            .expect("no other test puts a logger in place");
        });
    }

    /// The lines that the logger of the tests wrote for the test that calls this: the lines that
    /// were written on its thread, which is named after the test. The logger writes the name with
    /// a space on each side, so a test whose name is the start of another's does not read its
    /// lines.
    fn logged() -> Vec<String> {
        let thread = std::thread::current();
        let name = thread.name().expect("the thread of a test has its name");
        let text = String::from_utf8(LOGGED.lock().unwrap().clone()).unwrap();
        text.lines()
            .filter(|line| line.contains(&format!(" {name} ")))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn a_notice_is_logged_with_its_reason_and_how_many_events_it_is_about() {
        let (pictures, _heard) = pictures();
        pictures.began(3);
        pictures.noticed(notice(Reason::FrameUnknown, 4));
        let lines = logged();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("INFO"), "{lines:?}");
        assert!(lines[0].contains("reason=FrameUnknown"), "{lines:?}");
        assert!(lines[0].contains("count=4"), "{lines:?}");
    }

    #[test]
    fn a_notice_that_comes_before_the_window_is_known_is_not_logged() {
        let (pictures, _heard) = pictures();
        pictures.noticed(notice(Reason::Locked, 2));
        assert_eq!(logged(), Vec::<String>::new());
    }

    #[test]
    fn a_notice_that_comes_before_the_window_is_known_is_dropped_and_not_told_later() {
        let (pictures, mut heard) = pictures();
        pictures.noticed(notice(Reason::Locked, 2));
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
        pictures.began(3);
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
        pictures.noticed(notice(Reason::Locked, 1));
        assert_eq!(
            heard.try_recv(),
            Ok(noticed(3, WindowNoticeReason::Locked, 1))
        );
    }

    #[test]
    fn a_notice_is_told_in_its_order_among_the_sizes_of_the_picture() {
        let (pictures, mut heard) = pictures();
        pictures.said(1280, 800);
        pictures.began(3);
        pictures.noticed(notice(Reason::WindowResized, 5));
        pictures.said(800, 800);
        assert_eq!(heard.try_recv(), Ok(picture(3, 1280, 800)));
        assert_eq!(
            heard.try_recv(),
            Ok(noticed(3, WindowNoticeReason::Resized, 5))
        );
        assert_eq!(heard.try_recv(), Ok(picture(3, 800, 800)));
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn every_reason_the_other_device_gives_has_a_reason_for_the_screens() {
        use WindowNoticeReason as Shown;

        for (reason, shown) in [
            (Reason::ControlOff, Shown::ControlOff),
            (Reason::Paused, Shown::Paused),
            (Reason::Locked, Shown::Locked),
            (Reason::NotOnScreen, Shown::NotOnScreen),
            (Reason::TooFast, Shown::TooFast),
            (Reason::FrameUnknown, Shown::PictureUnknown),
            (Reason::PictureOutOfDate, Shown::OutOfDate),
            (Reason::WindowResized, Shown::Resized),
            (Reason::OutsideWindow, Shown::OutsideWindow),
            (Reason::Covered, Shown::Covered),
            (Reason::NoKeyboard, Shown::NoKeyboard),
            (Reason::AppWindowInFront, Shown::AppWindowInFront),
            (Reason::TooManyHeld, Shown::TooManyKeys),
            (Reason::NotPressed, Shown::NotPressed),
            (Reason::Malformed, Shown::Other),
            (Reason::Other, Shown::Other),
        ] {
            assert_eq!(Shown::from(reason), shown, "{reason:?}");
        }
    }

    #[tokio::test]
    async fn input_for_a_window_that_is_shared_from_here_or_unknown_is_not_taken() {
        let (events, _) = broadcast::channel(8);
        let sessions = Sessions::new(None, None, Questions::new(events.clone()), events);
        locked(&sessions.live).insert(7, Live::Sharing(Weak::new()));
        let touch = WindowInput::Pointer {
            x: 0.5,
            y: 0.5,
            modifiers: WindowModifiers::default(),
            action: WindowPointer::Down(WindowButton::Left),
        };
        assert!(!sessions.input(7, touch.clone()));
        assert!(!sessions.input(8, touch));
        // Nothing to let go of, and nothing goes wrong.
        sessions.release_input(7);
        sessions.release_input(8);
    }

    fn window(id: u64, role: WindowRole) -> WindowSession {
        WindowSession {
            id,
            role,
            device_id: "ab".repeat(32),
            device_name: "Studio Mac".to_owned(),
            display: false,
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
    fn a_notice_of_the_source_is_given_on_to_the_decoder_and_told_to_the_screens() {
        let (mut decoder, written, mut heard) = watching();
        decoder.source(SourceMessage::Notice(notice(Reason::NoKeyboard, 2)));
        assert_eq!(
            heard.try_recv(),
            Ok(noticed(5, WindowNoticeReason::NoKeyboard, 2))
        );
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
        let written = locked(&written);
        assert_eq!(written.len(), 1);
        assert!(written[0].starts_with("source Notice("), "{written:?}");
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
            (DeclineReason::Invalid, "could not read the offer"),
            (DeclineReason::Other, "did not take the window"),
        ] {
            let told = declined_window("Kitchen iPad", reason, false);
            assert!(told.contains("Kitchen iPad"), "{reason:?}: {told}");
            assert!(told.contains(why), "{reason:?}: {told}");
            assert!(!told.contains("screen"), "{reason:?}: {told}");
        }
    }

    #[test]
    fn what_the_person_is_told_of_a_screen_that_was_not_taken_calls_it_a_screen() {
        for (reason, why) in [
            (DeclineReason::Refused, "did not accept the screen"),
            (DeclineReason::Blocked, "does not take screens"),
            (DeclineReason::Unsupported, "cannot show screens"),
            (DeclineReason::Invalid, "offer of the screen"),
            (DeclineReason::Other, "did not take the screen"),
        ] {
            let told = declined_window("Kitchen iPad", reason, true);
            assert!(told.contains("Kitchen iPad"), "{reason:?}: {told}");
            assert!(told.contains(why), "{reason:?}: {told}");
            assert!(!told.contains("window"), "{reason:?}: {told}");
        }
    }

    #[test]
    fn a_device_that_is_busy_is_busy_with_a_window_or_a_screen_whichever_was_offered() {
        // What it is busy with is not known, and it need not be what was offered.
        for display in [false, true] {
            let told = declined_window("Kitchen iPad", DeclineReason::RateLimited, display);
            assert!(told.contains("Kitchen iPad"), "{display}: {told}");
            assert!(
                told.contains("busy with another window or screen"),
                "{display}: {told}"
            );
        }
    }
}
