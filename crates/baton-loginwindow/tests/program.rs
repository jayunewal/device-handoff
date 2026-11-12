// SPDX-License-Identifier: Apache-2.0

//! The program run the way launchd and the person run it: a folder of its own, a platform that the
//! test plays, a person at Terminal who is a script, and a phone that is a Baton app of its own.
//!
//! Pairing is done against the program that serves. In the real thing it is a run of its own
//! (`pair`), and the phone finds the one that serves afterwards by the key, the way it finds any
//! device that has restarted; here that run would listen on a port that nothing else knows.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    collections::BTreeSet,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use baton_app::{
    App, Device, ErrorKind as AppErrorKind, Event, Events, Options, Receiving, ScreenStarted,
    StartAccess, WindowButton, WindowEnd, WindowInput, WindowModifiers, WindowPointer, WindowRole,
    Windows,
    windows::{
        PickedWindow, PlatformError, Privacy, Screen, ScreenError, ScreenLink, SharingFailure,
        ShowLimits, WindowSharer, WindowViewer,
    },
};
use baton_core::{
    streaming::{
        DecodeError, Decoder, DecoderState, Picture, SourceEvents, SourceHandle, Takes, WindowOffer,
    },
    util::BoxFuture,
};
use baton_loginwindow::{ErrorKind, Person, Program, State, devices, name_this_mac, pair, serve};
use baton_proto::stream::Codec;
use baton_stream::{
    control::{Config, InputBatch, Report, SourceMessage, Thermal, Want},
    header::FrameHeader,
    input::{Button, Event as InputEvent, PointerAction},
    sender::LtrToken,
};
use tempfile::TempDir;
use tokio::sync::{mpsc, oneshot};

/// Long enough for a slow machine, short enough that a hang does not take the run with it.
const WAIT: Duration = Duration::from_secs(30);

/// Waits until `condition` holds.
async fn until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Waits for an event that `pick` likes, and returns what it made of it.
async fn next_matching<T>(events: &mut Events, mut pick: impl FnMut(Event) -> Option<T>) -> T {
    tokio::time::timeout(WAIT, async {
        loop {
            let event = events.next().await.expect("the app stopped while waiting");
            if let Some(found) = pick(event) {
                return found;
            }
        }
    })
    .await
    .expect("the event never came")
}

// The person at the Mac.

/// The person who runs the program in Terminal: what is said to them is kept, and what they answer
/// is whatever the test sends.
struct Typist {
    said: Mutex<Vec<String>>,
    copied: Mutex<Vec<String>>,
    /// Whether the Mac has a clipboard to copy to.
    clipboard: bool,
    answers: tokio::sync::Mutex<mpsc::UnboundedReceiver<Option<String>>>,
    typing: mpsc::UnboundedSender<Option<String>>,
}

impl Typist {
    fn new(clipboard: bool) -> Arc<Self> {
        let (typing, answers) = mpsc::unbounded_channel();
        Arc::new(Self {
            said: Mutex::new(Vec::new()),
            copied: Mutex::new(Vec::new()),
            clipboard,
            answers: tokio::sync::Mutex::new(answers),
            typing,
        })
    }

    /// What was said, a line at a time.
    fn said(&self) -> Vec<String> {
        self.said.lock().unwrap().clone()
    }

    fn said_has(&self, text: &str) -> bool {
        self.said().iter().any(|line| line.contains(text))
    }

    /// The ticket, once it was shown.
    async fn ticket(&self) -> String {
        let mut found = None;
        until("the ticket to be shown", || {
            found = self
                .said()
                .into_iter()
                .map(|line| line.trim().to_owned())
                .find(|line| line.to_ascii_uppercase().starts_with("BATON1:"));
            found.is_some()
        })
        .await;
        found.unwrap()
    }

    /// The person answers the question that was put; `None` is input that ended.
    fn answer(&self, answer: Option<&str>) {
        self.typing.send(answer.map(str::to_owned)).unwrap();
    }
}

impl Person for Typist {
    fn say(&self, text: &str) {
        self.said.lock().unwrap().push(text.to_owned());
    }

    fn copy(&self, text: &str) -> bool {
        self.copied.lock().unwrap().push(text.to_owned());
        self.clipboard
    }

    async fn ask(&self) -> Option<String> {
        self.answers.lock().await.recv().await.flatten()
    }
}

// The platform of the Mac: it gives a screen, and keeps what it was asked and what it was told.

struct Sharer {
    gives_screen: bool,
    unlockable: bool,
    /// What the screen was asked for as, each time.
    privacies: Mutex<Vec<Privacy>>,
    /// How many times the person was given a picker.
    picks: AtomicUsize,
    /// What the person at the phone did, as it reached the login window.
    inputs: Arc<Mutex<Vec<InputEvent>>>,
    /// How many sessions the platform was told the end of.
    ended: Arc<AtomicUsize>,
    /// The handle of the session that runs, which keeps it going.
    handle: Arc<Mutex<Option<Arc<SourceHandle>>>>,
}

impl Sharer {
    /// A platform that gives the main screen without anyone picking it.
    fn new() -> Arc<Self> {
        Self::building(true, false)
    }

    fn building(gives_screen: bool, unlockable: bool) -> Arc<Self> {
        Arc::new(Self {
            gives_screen,
            unlockable,
            privacies: Mutex::new(Vec::new()),
            picks: AtomicUsize::new(0),
            inputs: Arc::new(Mutex::new(Vec::new())),
            ended: Arc::new(AtomicUsize::new(0)),
            handle: Arc::new(Mutex::new(None)),
        })
    }

    fn privacies(&self) -> Vec<Privacy> {
        self.privacies.lock().unwrap().clone()
    }

    fn inputs(&self) -> Vec<InputEvent> {
        self.inputs.lock().unwrap().clone()
    }

    fn ended(&self) -> usize {
        self.ended.load(Ordering::Relaxed)
    }
}

impl WindowSharer for Sharer {
    fn control(&self) -> bool {
        true
    }

    fn pick(&self) -> BoxFuture<'static, Result<Option<Box<dyn PickedWindow>>, PlatformError>> {
        self.picks.fetch_add(1, Ordering::Relaxed);
        Box::pin(std::future::ready(Ok(None)))
    }

    fn can_give_screen(&self) -> bool {
        self.gives_screen
    }

    // A platform that can be unlocked can also lift what keeps its screen private, as that needs.
    fn can_lift(&self) -> bool {
        self.unlockable
    }

    fn can_be_unlocked(&self) -> bool {
        self.unlockable
    }

    fn screen(
        &self,
        privacy: Privacy,
    ) -> BoxFuture<'static, Result<Box<dyn PickedWindow>, ScreenError>> {
        self.privacies.lock().unwrap().push(privacy);
        let picked = Picked {
            inputs: Arc::clone(&self.inputs),
            ended: Arc::clone(&self.ended),
            handle: Arc::clone(&self.handle),
        };
        Box::pin(std::future::ready(Ok(
            Box::new(picked) as Box<dyn PickedWindow>
        )))
    }
}

/// The main display of the login window.
struct Picked {
    inputs: Arc<Mutex<Vec<InputEvent>>>,
    ended: Arc<AtomicUsize>,
    handle: Arc<Mutex<Option<Arc<SourceHandle>>>>,
}

impl PickedWindow for Picked {
    fn display(&self) -> bool {
        true
    }

    fn title(&self) -> String {
        String::new()
    }

    fn app(&self) -> String {
        String::new()
    }

    fn size(&self) -> (u32, u32) {
        (1920, 1080)
    }

    fn events(&mut self) -> Box<dyn SourceEvents> {
        Box::new(Hearing {
            inputs: Arc::clone(&self.inputs),
            ended: Arc::clone(&self.ended),
        })
    }

    fn prepare(&mut self, takes: &Takes) -> Option<Config> {
        takes.codecs.contains(&Codec::H264).then(|| Config {
            epoch: 0,
            codec: Codec::H264,
            width: takes.max_width.min(1920),
            height: takes.max_height.min(1080),
            fps: takes.max_fps.min(30),
        })
    }

    fn start(self: Box<Self>, handle: Arc<SourceHandle>, failure: SharingFailure) {
        drop(failure);
        *self.handle.lock().unwrap() = Some(Arc::clone(&handle));
        std::thread::spawn(move || {
            let deadline = Instant::now() + WAIT;
            let mut sent = 0;
            while sent < 3 && Instant::now() < deadline {
                if handle.begin().is_some() {
                    let picture = Picture {
                        bytes: vec![0x42; 2000],
                        referenced: true,
                        ltr: None,
                        capture_time: 16_000 * sent,
                    };
                    if handle.sent(picture).is_err() {
                        return;
                    }
                    sent += 1;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
    }
}

struct Hearing {
    inputs: Arc<Mutex<Vec<InputEvent>>>,
    ended: Arc<AtomicUsize>,
}

impl SourceEvents for Hearing {
    fn report(&mut self, _report: Report) {}

    fn want(&mut self, _want: Want) {}

    fn input(&mut self, batch: InputBatch, _arrived: Duration) {
        self.inputs.lock().unwrap().extend(batch.events);
    }

    fn held(&mut self, _tokens: Vec<LtrToken>) {}

    fn recovery(&mut self) {}

    fn ended(&mut self) {
        self.ended.fetch_add(1, Ordering::Relaxed);
    }
}

// The phone: a Baton app that shows the screens it is given.

struct Viewer {
    /// How many pictures were shown.
    shown: Arc<AtomicUsize>,
}

impl WindowViewer for Viewer {
    fn limits(&self) -> ShowLimits {
        ShowLimits {
            max_width: 1920,
            max_height: 1200,
            max_fps: 60,
            codecs: vec![Codec::H264],
        }
    }

    fn open(&self, _window: &WindowOffer, _link: ScreenLink) -> Result<Screen, PlatformError> {
        Ok(Screen::new(Box::new(Screening(Arc::clone(&self.shown))), 1))
    }
}

struct Screening(Arc<AtomicUsize>);

impl Decoder for Screening {
    fn decode(&mut self, _frame: &FrameHeader, _picture: Vec<u8>) -> Result<(), DecodeError> {
        Ok(())
    }

    fn show(&mut self, _number: u64) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    fn source(&mut self, _message: SourceMessage) {}

    fn state(&self) -> DecoderState {
        DecoderState {
            decode_us: 1000,
            thermal: Thermal::Nominal,
        }
    }

    fn ended(&mut self) {}
}

struct Phone {
    app: App,
    shown: Arc<AtomicUsize>,
    _dir: TempDir,
}

impl Phone {
    async fn start(name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        write_settings(dir.path(), Some(name));
        let shown = Arc::new(AtomicUsize::new(0));
        let viewer = Arc::new(Viewer {
            shown: Arc::clone(&shown),
        });
        let app = App::start_with(Options::new(dir.path()), Windows::none().showing(viewer))
            .await
            .unwrap();
        Self {
            app,
            shown,
            _dir: dir,
        }
    }

    fn id(&self) -> String {
        self.app.me().id
    }

    fn shown(&self) -> usize {
        self.shown.load(Ordering::Relaxed)
    }
}

/// The folder of a program, made as the program makes its own, inside `dir`, with settings that
/// keep it to this computer.
fn open_state(dir: &TempDir) -> State {
    let state = State::open(Some(dir.path().join("state"))).unwrap();
    write_settings(state.path(), None);
    state
}

/// Settings that keep a device to this computer.
fn write_settings(dir: &Path, name: Option<&str>) {
    let name = name.map_or_else(String::new, |name| format!("device_name = \"{name}\"\n\n"));
    std::fs::write(
        dir.join("config.toml"),
        format!(
            "{name}[network]\nrelay = \"disabled\"\nlan_discovery = false\nport_mapping = false\n"
        ),
    )
    .unwrap();
}

// The Mac, with the program running.

struct Mac {
    program: Program,
    state: State,
    sharer: Arc<Sharer>,
    dir: TempDir,
}

impl Mac {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let state = open_state(&dir);
        name_this_mac(&state, Some("Studio Mac")).unwrap();
        let sharer = Sharer::new();
        let program = Program::start(&state, Windows::none().sharing(sharer.clone()))
            .await
            .unwrap();
        Self {
            program,
            state,
            sharer,
            dir,
        }
    }

    fn id(&self) -> String {
        self.program.app().me().id
    }
}

/// What came of the person running `pair` at the Mac and a phone using the ticket.
struct Paired {
    /// What the program told the person.
    program: baton_loginwindow::Result<Device>,
    /// What the phone got.
    phone: baton_app::Result<Device>,
    /// What the phone showed for the person to compare, if it got that far.
    phone_code: Option<String>,
    typist: Arc<Typist>,
}

/// What `pair` says when it asks for the digits of the phone.
const ASKED: &str = "Type them here";

/// A line that the person types once the phone has asked to pair.
#[derive(Debug, Clone, Copy)]
enum Typed<'a> {
    /// The six digits that the phone shows, as it shows them.
    TheCode,
    /// The same six digits with nothing between them.
    TheCodeWithoutTheSpace,
    /// The six digits that the phone shows, with another last one.
    TheCodeWithAnotherLastDigit,
    /// The six digits that the phone shows, and then `more`.
    TheCodeAnd(&'a str),
    /// This line, whatever it is.
    Line(&'a str),
    /// Nothing more: the input ends.
    TheEnd,
}

impl Typed<'_> {
    /// Whether the line is made of the digits that the phone shows, so that the person can type
    /// it only after the phone has shown them.
    fn needs_the_code(self) -> bool {
        !matches!(self, Self::Line(_) | Self::TheEnd)
    }

    /// The line, with `code` as the phone shows it. `None` is the end of the input.
    fn line(self, code: &str) -> Option<String> {
        match self {
            Self::TheCode => Some(code.to_owned()),
            Self::TheCodeWithoutTheSpace => Some(code.replace(' ', "")),
            Self::TheCodeWithAnotherLastDigit => Some(with_another_last_digit(code)),
            Self::TheCodeAnd(more) => Some(format!("{code} {more}")),
            Self::Line(line) => Some(line.to_owned()),
            Self::TheEnd => None,
        }
    }
}

/// `code` with its last digit changed to another one.
fn with_another_last_digit(code: &str) -> String {
    let mut digits: Vec<char> = code.chars().collect();
    let last = digits.last_mut().expect("the code has digits");
    let digit = last.to_digit(10).expect("the code ends with a digit");
    *last = char::from_digit((digit + 1) % 10, 10).unwrap();
    digits.into_iter().collect()
}

/// The person runs `pair` at the Mac and the phone is given the ticket. Once the person has been
/// asked for the digits, they type `typed`, a line at a time.
async fn pair_with(mac: &Mac, phone: &Phone, typed: &[Typed<'_>]) -> Paired {
    pair_with_typing_ahead(mac, phone, &[], typed).await
}

/// As [`pair_with`], and the person has typed the lines of `ahead` before the phone asks, which a
/// terminal keeps for the program to read later, like the ones typed after.
async fn pair_with_typing_ahead(
    mac: &Mac,
    phone: &Phone,
    ahead: &[&str],
    typed: &[Typed<'_>],
) -> Paired {
    let typist = Typist::new(true);
    let app = mac.program.app().clone();
    let pairing = tokio::spawn({
        let typist = Arc::clone(&typist);
        async move { pair(&app, &*typist, std::future::pending()).await }
    });
    let ticket = typist.ticket().await;
    for &line in ahead {
        typist.answer(Some(line));
    }
    let code = Arc::new(Mutex::new(None));
    let joining = tokio::spawn({
        let app = phone.app.clone();
        let code = Arc::clone(&code);
        async move {
            app.join_pairing(&ticket, |shown| *code.lock().unwrap() = Some(shown))
                .await
        }
    });
    until("the person to be asked", || typist.said_has(ASKED)).await;
    if typed.iter().any(|typed| typed.needs_the_code()) {
        until("the phone to show its code", || {
            code.lock().unwrap().is_some()
        })
        .await;
    }
    let shown = code.lock().unwrap().clone().unwrap_or_default();
    for typed in typed {
        typist.answer(typed.line(&shown).as_deref());
    }
    let program = pairing.await.unwrap();
    let phone = joining.await.unwrap();
    let phone_code = code.lock().unwrap().clone();
    Paired {
        program,
        phone,
        phone_code,
        typist,
    }
}

/// The phone is paired with the program, and the person typed the digits that it shows.
async fn paired(mac: &Mac, phone: &Phone) {
    let done = pair_with(mac, phone, &[Typed::TheCode]).await;
    done.program.unwrap();
    done.phone.unwrap();
}

/// The names of what is in `dir`, which is all that is kept of the program.
fn files_in(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

fn touch(action: WindowPointer) -> WindowInput {
    WindowInput::Pointer {
        x: 0.25,
        y: 0.75,
        modifiers: WindowModifiers::default(),
        action,
    }
}

fn key(code: u16, down: bool) -> WindowInput {
    WindowInput::Key {
        code,
        down,
        modifiers: WindowModifiers::default(),
        repeat: false,
        text: None,
    }
}

// Pairing, in Terminal.

#[tokio::test]
async fn a_phone_whose_digits_the_person_types_is_paired_and_may_open_the_login_window_and_nothing_else()
 {
    let mac = Mac::start().await;
    let phone = Phone::start("Pocket Phone").await;

    let done = pair_with(&mac, &phone, &[Typed::TheCode]).await;

    // The phone is paired, and it knows the program by the name that says where it runs.
    let at_mac = done.program.unwrap();
    let at_phone = done.phone.unwrap();
    assert_eq!(at_mac.id, phone.id());
    assert_eq!(at_mac.name, "Pocket Phone");
    assert_eq!(at_phone.id, mac.id());
    assert_eq!(at_phone.name, "Studio Mac (login window)");

    // What it may do is set by the pairing: the main screen with control, which the list says too.
    assert_eq!(
        mac.program.app().start_access(&phone.id()).unwrap(),
        StartAccess::Control
    );
    let listed = devices::list(&mac.state).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].device.id, phone.id());
    assert_eq!(listed[0].start, StartAccess::Control);

    // The person was asked for the digits and shown nothing that the device sent, but the system
    // it says it runs. Not the digits, which are read off the phone, and not its name, which a
    // stranger who holds the ticket could fill with them.
    let said = done.typist.said();
    let code = done.phone_code.expect("the phone showed no code");
    assert!(done.typist.said_has(ASKED), "{said:?}");
    let asked_at = said
        .iter()
        .position(|line| line.contains("A device wants to pair"))
        .expect("the person was not told that a device asks");
    let paired_at = said
        .iter()
        .position(|line| line.starts_with("Paired with"))
        .expect("the person was not told that the device is paired");
    let question = &said[asked_at..paired_at];
    let digits = code.replace(' ', "");
    assert!(
        question
            .iter()
            .all(|line| !line.contains(&code) && !line.contains(&digits)),
        "the digits were shown at the Mac: {said:?}"
    );
    assert!(
        question
            .iter()
            .all(|line| { !line.contains("Pocket Phone") && !line.contains(&at_mac.fingerprint) }),
        "something that the device sent was shown with the question: {said:?}"
    );
    // Once it is paired, the person is told who it is.
    assert!(
        said[paired_at].contains("Pocket Phone") && said[paired_at].contains(&at_mac.fingerprint),
        "{said:?}"
    );
    // The ticket was on the clipboard as well, and the person was told how to take the phone off.
    assert_eq!(done.typist.copied.lock().unwrap().len(), 1);
    assert!(done.typist.said_has("on the clipboard"));
    let prefix: String = phone.id().chars().take(12).collect();
    assert!(
        done.typist
            .said_has(&format!("baton-loginwindow forget {prefix}")),
        "{said:?}"
    );
    // Nothing that was said can move the cursor or change the screen of a person's Terminal.
    assert!(
        said.iter().all(|line| !line.chars().any(char::is_control)),
        "{said:?}"
    );
}

#[tokio::test]
async fn the_digits_are_taken_with_or_without_the_space_that_the_phone_shows_in_them() {
    let mac = Mac::start().await;
    let phone = Phone::start("Pocket Phone").await;

    let done = pair_with(&mac, &phone, &[Typed::TheCodeWithoutTheSpace]).await;

    done.program.unwrap();
    done.phone.unwrap();
    assert_eq!(devices::list(&mac.state).unwrap().len(), 1);
}

#[tokio::test]
async fn anything_but_the_digits_that_the_phone_shows_pairs_nothing() {
    let cases = [
        // What the old question asked for, and what another one might.
        Typed::Line("yes"),
        Typed::Line("y"),
        Typed::Line("YES"),
        Typed::Line("no"),
        Typed::Line("yes please"),
        // Digits, but not those.
        Typed::Line("123"),
        Typed::TheCodeWithAnotherLastDigit,
        Typed::TheCodeAnd("yes"),
    ];
    for typed in cases {
        let mac = Mac::start().await;
        let phone = Phone::start("Pocket Phone").await;

        let done = pair_with(&mac, &phone, &[typed]).await;

        let error = done.program.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Pairing, "{typed:?}: {error}");
        assert!(done.phone.is_err(), "{typed:?}");
        assert!(
            done.typist.said_has("That is not what the phone shows."),
            "{typed:?}"
        );
        assert!(devices::list(&mac.state).unwrap().is_empty(), "{typed:?}");
        assert!(mac.program.app().devices().is_empty(), "{typed:?}");
    }
}

#[tokio::test]
async fn an_input_that_ends_pairs_nothing() {
    let mac = Mac::start().await;
    let phone = Phone::start("Pocket Phone").await;

    let done = pair_with(&mac, &phone, &[Typed::TheEnd]).await;

    let error = done.program.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Pairing, "{error}");
    assert!(done.phone.is_err());
    assert!(done.typist.said_has("Nothing more can be typed."));
    assert!(
        devices::list(&mac.state).unwrap().is_empty(),
        "a device was kept"
    );
    assert!(
        mac.program.app().devices().is_empty(),
        "a device was paired"
    );
}

#[tokio::test]
async fn a_line_typed_before_the_phone_asks_pairs_nothing() {
    // A terminal keeps what is typed while the program waits, and gives it to the next question:
    // a `yes` typed then, by habit or by hurry, was waiting for the first device that asked.
    for ahead in [&["yes"][..], &["y"], &["Yes", "yes"], &["", "yes"]] {
        let mac = Mac::start().await;
        let phone = Phone::start("Pocket Phone").await;

        let done = pair_with_typing_ahead(&mac, &phone, ahead, &[]).await;

        let error = done.program.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Pairing, "{ahead:?}: {error}");
        assert!(done.phone.is_err(), "{ahead:?}");
        assert!(
            done.typist.said_has("That is not what the phone shows."),
            "{ahead:?}"
        );
        assert!(devices::list(&mac.state).unwrap().is_empty(), "{ahead:?}");
        assert!(mac.program.app().devices().is_empty(), "{ahead:?}");
    }
}

#[tokio::test]
async fn a_return_is_not_an_answer_before_the_phone_asks_or_after() {
    let mac = Mac::start().await;
    let phone = Phone::start("Pocket Phone").await;

    let done = pair_with_typing_ahead(
        &mac,
        &phone,
        &["", "   "],
        &[Typed::Line(""), Typed::Line("  "), Typed::TheCode],
    )
    .await;

    done.program.unwrap();
    done.phone.unwrap();
    assert!(
        !done.typist.said_has("not what the phone shows"),
        "a Return was taken for an answer"
    );
    assert_eq!(devices::list(&mac.state).unwrap().len(), 1);
}

#[tokio::test]
async fn a_pairing_that_is_stopped_withdraws_the_ticket_and_pairs_nothing() {
    let mac = Mac::start().await;
    let phone = Phone::start("Pocket Phone").await;
    let mut events = mac.program.app().events();
    let typist = Typist::new(false);
    let (stop, stopped) = oneshot::channel::<()>();
    let app = mac.program.app().clone();
    let pairing = tokio::spawn({
        let typist = Arc::clone(&typist);
        async move {
            pair(&app, &*typist, async {
                let _ = stopped.await;
            })
            .await
        }
    });
    let ticket = typist.ticket().await;
    assert!(
        !typist.said_has("clipboard"),
        "a person who has no clipboard is not told that the ticket is on it"
    );

    stop.send(()).unwrap();

    let error = pairing.await.unwrap().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Pairing, "{error}");
    next_matching(&mut events, |event| {
        matches!(event, Event::OfferEnded(_)).then_some(())
    })
    .await;
    assert!(phone.app.join_pairing(&ticket, |_| {}).await.is_err());
    assert!(
        devices::list(&mac.state).unwrap().is_empty(),
        "a device was paired"
    );
}

// The platform, and the folder.

#[tokio::test]
async fn a_platform_that_is_not_what_the_program_needs_is_refused_and_the_folder_is_let_go() {
    let dir = tempfile::tempdir().unwrap();
    let state = open_state(&dir);
    let viewer = Arc::new(Viewer {
        shown: Arc::new(AtomicUsize::new(0)),
    });
    let platforms = [
        ("no platform at all", Windows::none()),
        (
            "a platform that cannot give the screen unasked",
            Windows::none().sharing(Sharer::building(false, false)),
        ),
        (
            "a platform that shows the windows of other devices",
            Windows::none().sharing(Sharer::new()).showing(viewer),
        ),
        (
            "a platform that can unlock the screen",
            Windows::none().sharing(Sharer::building(true, true)),
        ),
    ];
    for (what, windows) in platforms {
        let Err(error) = Program::start(&state, windows).await else {
            panic!("{what} was accepted");
        };
        assert_eq!(error.kind(), ErrorKind::Platform, "{what}: {error}");
    }

    // None of them kept the folder: a platform that is right starts, and lets go again.
    let program = Program::start(&state, Windows::none().sharing(Sharer::new()))
        .await
        .unwrap();
    program.stop().await;
}

#[tokio::test]
async fn the_program_serves_until_it_is_told_to_stop_and_then_lets_go_of_its_folder() {
    let dir = tempfile::tempdir().unwrap();
    let state = open_state(&dir);

    // Told at once: it starts, and stops.
    for _ in 0..2 {
        let windows = Windows::none().sharing(Sharer::new());
        tokio::time::timeout(WAIT, serve(&state, windows, std::future::ready(())))
            .await
            .expect("the program did not stop")
            .unwrap();
    }

    // A program that is running holds the folder, and a second one is told so.
    let running = Program::start(&state, Windows::none().sharing(Sharer::new()))
        .await
        .unwrap();
    let second = Program::start(&state, Windows::none().sharing(Sharer::new()))
        .await
        .unwrap_err();
    assert_eq!(second.kind(), ErrorKind::Busy, "{second}");
    running.stop().await;
}

// What a phone that was paired can do.

#[tokio::test]
async fn a_paired_phone_starts_the_screen_and_what_it_does_there_reaches_the_login_window() {
    let mac = Mac::start().await;
    let phone = Phone::start("Pocket Phone").await;
    paired(&mac, &phone).await;
    let mut events = mac.program.app().events();

    let ScreenStarted::Started(session) = phone.app.start_screen(&mac.id(), true).await.unwrap()
    else {
        panic!("the screen was not started");
    };

    // The phone got the main display, with control. `session.private` is this fake platform's
    // own `PickedWindow::private()`, which it never overrides, so it reads false here whatever
    // privacy was asked for; it is not a sign that the screen was kept open. The screen really
    // was asked for as `Privacy::Private`, since control was given, the same as `App` asks for
    // any other screen a Baton shares (`docs/adr/0020-screen-sharing-at-the-login-window.md`'s
    // V9, not yet answered): a real platform at the login window has to refuse that somehow, or
    // `App` has to ask for something else, before the login window can keep nobody signed in to
    // be held out of their own sign-in. Nothing here answers that yet.
    assert!(session.display && session.control, "{session:?}");
    assert_eq!(mac.sharer.privacies(), [Privacy::Private]);
    assert_eq!(mac.sharer.picks.load(Ordering::Relaxed), 0);
    let shared = next_matching(&mut events, |event| match event {
        Event::WindowStarted(session) => Some(session),
        _ => None,
    })
    .await;
    assert_eq!(shared.role, WindowRole::Sharing);
    assert_eq!(shared.device_name, "Pocket Phone");
    assert!(
        shared.display && shared.control && !shared.private,
        "{shared:?}"
    );

    // A touch and a key at the phone reach the login window.
    until("a picture to be shown at the phone", || phone.shown() > 0).await;
    assert!(
        phone
            .app
            .window_input(session.id, touch(WindowPointer::Down(WindowButton::Left)))
    );
    assert!(phone.app.window_input(session.id, key(4, true)));
    assert!(phone.app.window_input(session.id, key(4, false)));
    until("the login window to hear all three", || {
        mac.sharer.inputs().len() == 3
    })
    .await;
    let heard = mac.sharer.inputs();
    assert!(
        matches!(
            heard[0],
            InputEvent::Pointer {
                action: PointerAction::Down(Button::Left),
                ..
            }
        ),
        "{heard:?}"
    );
    assert!(matches!(
        heard[1],
        InputEvent::Key {
            code: 4,
            down: true,
            ..
        }
    ));

    // The person at the phone ends it, and the platform is told.
    assert!(phone.app.end_window(session.id));
    until("the platform to be told", || mac.sharer.ended() == 1).await;
}

#[tokio::test]
async fn stopping_the_program_ends_the_session_of_the_phone() {
    let mac = Mac::start().await;
    let phone = Phone::start("Pocket Phone").await;
    paired(&mac, &phone).await;
    let mut events = phone.app.events();
    let ScreenStarted::Started(session) = phone.app.start_screen(&mac.id(), true).await.unwrap()
    else {
        panic!("the screen was not started");
    };

    mac.program.stop().await;

    let ended = next_matching(&mut events, |event| match event {
        Event::WindowEnded { id, end } if id == session.id => Some(end),
        _ => None,
    })
    .await;
    // However the phone is told, it is told: the screen does not stay on the phone, frozen.
    assert!(
        matches!(ended, WindowEnd::Lost | WindowEnd::OtherDevice),
        "{ended:?}"
    );
}

#[tokio::test]
async fn the_program_takes_no_links_text_or_files_and_keeps_nothing_of_them() {
    let mac = Mac::start().await;
    let phone = Phone::start("Pocket Phone").await;
    paired(&mac, &phone).await;
    let kept = files_in(mac.state.path());
    let note = tempfile::tempdir().unwrap();
    std::fs::write(note.path().join("note.txt"), "hello").unwrap();

    let link = phone
        .app
        .send_link(&mac.id(), "https://example.com/path", None)
        .await
        .unwrap_err();
    assert_eq!(link.kind(), AppErrorKind::Declined, "{link}");
    let text = phone.app.send_text(&mac.id(), "hello").await.unwrap_err();
    assert_eq!(text.kind(), AppErrorKind::Declined, "{text}");
    let sending = phone
        .app
        .send_files(&mac.id(), vec![note.path().join("note.txt")])
        .await
        .unwrap();
    let files = tokio::time::timeout(WAIT, sending.outcome())
        .await
        .expect("the files were neither taken nor declined")
        .unwrap_err();
    assert_eq!(files.kind(), AppErrorKind::Declined, "{files}");

    assert_eq!(mac.program.app().receiving(), Receiving::Off);
    assert!(
        mac.program.app().inbox().unwrap().is_empty(),
        "something was kept in the inbox"
    );
    assert_eq!(files_in(mac.state.path()), kept, "something was written");
}

#[tokio::test]
async fn a_phone_that_is_not_on_the_list_cannot_start_the_screen() {
    let mac = Mac::start().await;
    let phone = Phone::start("Pocket Phone").await;
    paired(&mac, &phone).await;
    // The Mac drops it from its list, and the phone does not know.
    mac.program.app().unpair(&phone.id()).unwrap();

    let started = tokio::time::timeout(WAIT, phone.app.start_screen(&mac.id(), true))
        .await
        .expect("the phone was left waiting");

    // The Mac closed the connection, saying that the phone is not paired with it.
    let error = started.unwrap_err();
    assert!(error.message().contains("not paired"), "{error}");
    assert!(
        mac.sharer.privacies().is_empty(),
        "the platform was asked for the screen"
    );
}

// The list.

#[tokio::test]
async fn the_list_says_what_each_device_may_do_and_forget_takes_one_off_it() {
    let mac = Mac::start().await;
    assert!(
        devices::list(&mac.state).unwrap().is_empty(),
        "a device was paired"
    );
    let phone = Phone::start("Pocket Phone").await;
    paired(&mac, &phone).await;
    let Mac {
        program,
        state,
        dir: _dir,
        ..
    } = mac;

    // Reading the list needs no lock, and changing it is refused while the program runs.
    let listed = devices::list(&state).unwrap();
    assert_eq!(listed.len(), 1);
    let id = phone.id();
    let description = listed[0].describe();
    assert!(description.contains("Pocket Phone"), "{description}");
    assert!(description.contains(&id), "{description}");
    assert!(
        description.contains(&listed[0].device.fingerprint),
        "{description}"
    );
    assert!(
        description.contains("look at the login window and use it"),
        "{description}"
    );
    let busy = devices::forget(&state, &id[..12]).unwrap_err();
    assert_eq!(busy.kind(), ErrorKind::Busy, "{busy}");

    program.stop().await;

    // A few letters, or letters that no device has, name nothing.
    for which in [&id[..5], "zzzzzz", "  "] {
        let error = devices::forget(&state, which).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::NoDevice, "{which:?}: {error}");
        assert_eq!(devices::list(&state).unwrap().len(), 1, "{which:?}");
    }
    // The start of the id, in any case, names the device, and it is gone.
    let gone = devices::forget(&state, &id[..12].to_ascii_uppercase()).unwrap();
    assert_eq!(gone.id, id);
    assert_eq!(gone.name, "Pocket Phone");
    assert!(
        devices::list(&state).unwrap().is_empty(),
        "the device is still on the list"
    );
    let again = devices::forget(&state, &id[..12]).unwrap_err();
    assert_eq!(again.kind(), ErrorKind::NoDevice, "{again}");
}
