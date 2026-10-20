// SPDX-License-Identifier: Apache-2.0

//! What the person at the other device does to a window that is shared with control.
//!
//! The session calls [`SourceEvents`] with its lock held, so none of its methods may wait, and
//! putting an event to a window asks the platform two questions that take a moment: what the Mac
//! is doing around the window, and to post the event. So the events only go into a queue
//! ([`Forward`]), and a thread of its own ([`Worker`]) takes them out, one batch after the other:
//!
//! 1. It asks the platform for the surroundings of the window, once for the whole batch.
//! 2. Each event goes through the [`Gate`], which holds the rules of ADR 0009. What it lets
//!    through is posted to the window, and what it drops is counted for the viewer, with the
//!    reason, by the session.
//! 3. If the session is paused, or the Mac is locked or the window is gone from the screen, what
//!    is still held down is released.
//!
//! The same thread runs `start`, `ended` and `release` of the window, which is the one thread
//! that the header promises for all the calls of a window. It holds the window from the start and
//! lets go of it when the session is over, so a platform that is in the middle of a post when the
//! session ends is waited for, and the release of a press that was posted is not lost.
//!
//! A press that was posted is always released: by the viewer's own release, or, when that does not
//! come, by the thread. It releases everything that is held when the session is paused, when the
//! Mac locks or the window leaves the screen, when the viewer has not reported for [`Timing::quiet`],
//! at the place in the queue where events were lost, and when the session is over.
//!
//! A thread with nothing held down sleeps until something wakes it. While something is held down
//! it looks at the window once every [`Timing::poll`], since a Mac that locks or a window that goes
//! away says nothing to the thread.
//!
//! Locks: the gate is taken before the lock of the session and never after it. [`SourceLink::sent`]
//! holds the gate across the call that sends a picture, so that a frame is known to the gate by the
//! time anyone can name it; the thread takes the gate for one event at a time and holds nothing
//! while it calls the platform or tells the session what was dropped; and [`Forward`], which runs
//! with the lock of the session held, takes the queue's lock only.
//!
//! Nothing here logs: what a person types and where they tap is theirs.
//!
//! [`SourceLink::sent`]: crate::sharer::SourceLink::sent

use std::{
    collections::VecDeque,
    mem,
    sync::{
        Arc, Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::SyncSender,
    },
    time::{Duration, Instant},
};

use baton_core::streaming::SourceEvents;
use baton_stream::{
    control::{InputBatch, Report, Want},
    gate::{Context, Decision, Gate, Reason, Rect},
    sender::LtrToken,
};

use crate::{
    platform::{Failure, PickedHandle, Surroundings},
    registry::{Links, lock},
};

/// How many batches wait for the thread. A viewer sends a batch for each event when they come
/// slowly, and at most 240 events in a second, so a queue that is full means that the platform has
/// not answered for a second or more.
const QUEUE_BATCHES: usize = 256;

/// How many of the lost events the viewer is told about at a time. A count is cheap, but a loop of
/// a million of them would not be.
const MOST_LOST_TOLD: usize = 1024;

/// How long the thread waits, and how long a press may stay down without word from the viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Timing {
    /// How often the thread looks at what is held down, when something is and nothing else wakes
    /// it.
    pub(crate) poll: Duration,
    /// How long the viewer may go without a report while something is held down.
    pub(crate) quiet: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            poll: Duration::from_secs(1),
            quiet: Duration::from_secs(10),
        }
    }
}

/// What waits in the queue, in the order that it happened.
#[derive(Debug, Clone, PartialEq)]
enum Queued {
    /// A batch of the viewer, with the time that it arrived on the clock of the session, and the
    /// moment that it was put in the queue.
    Batch(InputBatch, Duration, Instant),
    /// This many events were refused because the queue was full, and what the thread does with
    /// them comes after the batches ahead of this: one of the events lost may have been the release
    /// of a press that one of those batches began.
    Lost(usize),
}

/// How far the session has got, as the thread is told.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// The session runs.
    #[default]
    Running,
    /// The session is over: nothing more is put to the window, and the thread is to be told.
    Over,
    /// The session is over, and the thread was told so, once.
    OverTold,
    /// The window is to be told that the session is over, and let go of.
    Closed,
}

/// What the thread has to look at.
#[derive(Default)]
struct Waiting {
    queue: VecDeque<Queued>,
    /// Something else changed: the session was paused, or went on.
    nudged: bool,
    phase: Phase,
}

/// What the thread does next.
#[derive(Debug, PartialEq)]
enum Step {
    /// Put these events to the window.
    Batch(InputBatch, Duration, Instant),
    /// This many events were lost.
    Lost(usize),
    /// The session is over.
    Over,
    /// Tell the window, and let go of it.
    Close,
    /// Nothing arrived: look at what is held down.
    Look,
}

impl Waiting {
    fn step(&mut self) -> Option<Step> {
        match self.phase {
            Phase::Closed => return Some(Step::Close),
            Phase::Over | Phase::OverTold => {
                self.queue.clear();
                self.nudged = false;
                let told = mem::replace(&mut self.phase, Phase::OverTold) == Phase::OverTold;
                return (!told).then_some(Step::Over);
            }
            Phase::Running => {}
        }
        match self.queue.pop_front() {
            Some(Queued::Batch(batch, arrived, queued)) => {
                Some(Step::Batch(batch, arrived, queued))
            }
            Some(Queued::Lost(lost)) => Some(Step::Lost(lost)),
            None => mem::take(&mut self.nudged).then_some(Step::Look),
        }
    }
}

/// What the session and the platform's thread share for one window: the queue of events and the
/// rules that decide what they do.
pub(crate) struct Input {
    waiting: Mutex<Waiting>,
    wake: Condvar,
    gate: Mutex<Gate>,
    paused: AtomicBool,
    began: Instant,
    /// When the viewer last reported, in microseconds since `began`.
    heard: AtomicU64,
    timing: Timing,
}

impl Input {
    pub(crate) fn new(timing: Timing) -> Arc<Self> {
        Arc::new(Self {
            waiting: Mutex::new(Waiting::default()),
            wake: Condvar::new(),
            gate: Mutex::new(Gate::new()),
            paused: AtomicBool::new(false),
            began: Instant::now(),
            heard: AtomicU64::new(0),
            timing,
        })
    }

    /// The rules, to record what a picture showed. Held across the call that sends the picture,
    /// see the module's docs.
    pub(crate) fn gate(&self) -> MutexGuard<'_, Gate> {
        lock(&self.gate)
    }

    /// The session is paused: what is held down is released, and nothing is put to the window
    /// until it goes on.
    pub(crate) fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
        self.nudge();
    }

    /// The session goes on. What was held down was let go of when it was paused, so the thread has
    /// nothing to look at.
    pub(crate) fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }

    fn nudge(&self) {
        lock(&self.waiting).nudged = true;
        self.wake.notify_one();
    }

    /// The viewer reported.
    fn heard_now(&self) {
        let micros = u64::try_from(self.began.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.heard.store(micros, Ordering::SeqCst);
    }

    /// How long ago the viewer last reported.
    fn silence(&self) -> Duration {
        let heard = Duration::from_micros(self.heard.load(Ordering::SeqCst));
        self.began.elapsed().saturating_sub(heard)
    }

    /// Queues a batch for the thread. A batch that finds the queue full is lost, and the thread
    /// releases what is held when it comes to the place where the batch would have been, since it
    /// cannot tell what was in it. A place like that is a mark in the queue, and the batches lost
    /// one after the other are one mark, so the queue is never longer than `QUEUE_BATCHES` places
    /// and one more.
    fn push(&self, batch: InputBatch, arrived: Duration) {
        let mut waiting = lock(&self.waiting);
        if waiting.phase != Phase::Running {
            return;
        }
        if waiting.queue.len() < QUEUE_BATCHES {
            waiting
                .queue
                .push_back(Queued::Batch(batch, arrived, Instant::now()));
        } else if !batch.events.is_empty() {
            let lost = batch.events.len();
            match waiting.queue.back_mut() {
                Some(Queued::Lost(sum)) => *sum = sum.saturating_add(lost),
                _ => waiting.queue.push_back(Queued::Lost(lost)),
            }
        }
        drop(waiting);
        self.wake.notify_one();
    }

    /// The session is over.
    fn over(&self) {
        let mut waiting = lock(&self.waiting);
        if waiting.phase == Phase::Running {
            waiting.phase = Phase::Over;
        }
        drop(waiting);
        self.wake.notify_one();
    }

    /// The window is to be told that the session is over, and let go of.
    fn close(&self) {
        lock(&self.waiting).phase = Phase::Closed;
        self.wake.notify_one();
    }

    /// Waits for the next thing to do. While something is held down that is for at most
    /// [`Timing::poll`], and a wait that comes to its end is a reason to look at what is held; a
    /// window with nothing held down waits until it is woken, and costs nothing meanwhile.
    fn next(&self) -> Step {
        // Only the thread that asks presses anything, so what is held now is held until it acts.
        // The gate is let go of before the queue is locked, see the module's docs.
        let holding = self.gate().is_pressing();
        let mut waiting = lock(&self.waiting);
        loop {
            if let Some(step) = waiting.step() {
                return step;
            }
            if holding {
                let (mut waiting, _) = self
                    .wake
                    .wait_timeout(waiting, self.timing.poll)
                    .unwrap_or_else(PoisonError::into_inner);
                return waiting.step().unwrap_or(Step::Look);
            }
            waiting = self
                .wake
                .wait(waiting)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// What the session tells the platform, as the session hears it: the events of the person go to
/// the queue, the reports of the viewer say that someone is there, and the end of the session says
/// that nothing more is to be put to the window.
///
/// The rest the platform has no use for: it asks `begin` and `recovery_pending` on its own clock,
/// and the picture it makes is not told what the viewer wants.
pub(crate) struct Forward {
    input: Arc<Input>,
}

impl Forward {
    pub(crate) fn new(input: Arc<Input>) -> Self {
        Self { input }
    }
}

impl SourceEvents for Forward {
    fn report(&mut self, _: Report) {
        self.input.heard_now();
    }

    fn want(&mut self, _: Want) {}

    fn input(&mut self, batch: InputBatch, arrived: Duration) {
        self.input.push(batch, arrived);
    }

    fn held(&mut self, _: Vec<LtrToken>) {}

    fn recovery(&mut self) {}

    fn ended(&mut self) {
        self.input.over();
    }
}

/// Closes the link of the session and then tells the thread to let the window go, when it is
/// dropped. The task that waits for the end of the session holds it, so both happen whether the
/// task finishes or is dropped with its runtime. The link is closed first, so that the platform,
/// which is told that the session is over by the thread, finds the link closed by then.
pub(crate) struct Closing {
    links: Arc<Links>,
    number: u64,
    input: Arc<Input>,
}

impl Closing {
    pub(crate) fn new(links: Arc<Links>, number: u64, input: Arc<Input>) -> Self {
        Self {
            links,
            number,
            input,
        }
    }
}

impl Drop for Closing {
    fn drop(&mut self) {
        self.links.remove(self.number);
        self.input.close();
    }
}

/// Tells the session that an event was dropped, and why. Called with no lock of the crate held.
pub(crate) type Dropped = Box<dyn Fn(Reason) + Send>;

/// The thread that puts the events to the window, and calls it from `start` to `release`.
pub(crate) struct Worker {
    input: Arc<Input>,
    window: PickedHandle,
    dropped: Dropped,
    /// Where the window was when the platform last said.
    at: Rect,
}

impl Worker {
    pub(crate) fn new(input: Arc<Input>, window: PickedHandle, dropped: Dropped) -> Self {
        Self {
            input,
            window,
            dropped,
            at: Rect::default(),
        }
    }

    /// Starts the window with `link`, tells `started` how that went, and if it went well serves the
    /// window until it is closed. A window that did not start is let go of without `ended`.
    pub(crate) fn run(mut self, link: u64, started: &SyncSender<Result<(), Failure>>) {
        let result = self.window.start(link);
        let failed = result.is_err();
        // A start that nobody waits for any more is as good as one that failed.
        if started.send(result).is_err() || failed {
            return;
        }
        // The session began a moment ago, and its viewer has not had the time to report.
        self.input.heard_now();
        loop {
            match self.input.next() {
                Step::Batch(batch, arrived, queued) => self.batch(&batch, arrived, queued),
                Step::Lost(lost) => self.lost(lost),
                Step::Over => self.release_held(),
                Step::Look => self.look(),
                Step::Close => {
                    self.release_held();
                    self.window.ended();
                    return;
                }
            }
        }
    }

    /// Puts the events of a batch to the window, as far as the rules allow.
    ///
    /// The rules judge an event at the time that it is put to the window, on the clock of the
    /// session: when it arrived and for as long as it waited in the queue. A thread that was held
    /// up by the platform does not put an event to a window that changed while it waited as if it
    /// had not.
    fn batch(&mut self, batch: &InputBatch, arrived: Duration, queued: Instant) {
        let now = arrived.saturating_add(queued.elapsed());
        let paused = self.input.paused.load(Ordering::SeqCst);
        let around = self.window.surroundings();
        // What the Mac could not say is not known to be fine: the window is not known to be on
        // the screen, so nothing is put to it but the release of what it was given.
        let known = around.is_some();
        let around = around.unwrap_or_else(Surroundings::unknown);
        if known {
            self.at = around.window;
        }
        let context = Context {
            now,
            control: true,
            paused,
            locked: around.locked,
            on_screen: around.on_screen,
            window: around.window,
            in_front: &around.fronts,
            keyboard: around.keyboard,
        };
        for event in &batch.events {
            let decision = self.input.gate().handle(&context, event);
            match decision {
                Decision::Post(post) => {
                    if !self.window.post(&post) {
                        // The platform took it for something it could not do. The viewer is told
                        // as for a rule that this version does not know.
                        (self.dropped)(Reason::Other);
                    }
                }
                Decision::Drop(reason) => (self.dropped)(reason),
            }
        }
        if paused || around.locked || !around.on_screen {
            self.release();
        }
    }

    /// Events were refused because the thread did not keep up, and some of them may have been the
    /// release of a press: everything is released, and the viewer is told.
    fn lost(&mut self, lost: usize) {
        self.release_held();
        for _ in 0..lost.min(MOST_LOST_TOLD) {
            (self.dropped)(Reason::TooFast);
        }
    }

    /// Nothing arrived. If something is held down, whether it still may be is looked at again.
    fn look(&mut self) {
        if !self.input.gate().is_pressing() {
            return;
        }
        let paused = self.input.paused.load(Ordering::SeqCst);
        let silent = self.input.silence() > self.input.timing.quiet;
        let around = self.window.surroundings();
        if let Some(around) = &around {
            self.at = around.window;
        }
        let fine = around
            .as_ref()
            .is_some_and(|around| !around.locked && around.on_screen);
        if paused || silent || !fine {
            self.release();
        }
    }

    /// Releases everything that was pressed, in the window as it is now if the platform can say
    /// where that is.
    fn release_held(&mut self) {
        if !self.input.gate().is_pressing() {
            return;
        }
        if let Some(around) = self.window.surroundings() {
            self.at = around.window;
        }
        self.release();
    }

    /// Releases everything that was pressed, in the window as it was when the platform last said.
    fn release(&mut self) {
        let posts = self.input.gate().release_all(self.at);
        for post in posts {
            // Nothing to be done if the platform cannot: the gate does not hold the press any
            // more either way.
            let _ = self.window.post(&post);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::{
        ptr::NonNull,
        sync::mpsc,
        thread::{self, JoinHandle},
    };

    use baton_stream::{
        control::Thermal,
        gate::{Geometry, Keyboard, Point, Post, Size},
        input::{Button, Event, MAX_BATCH, Modifiers, NO_KEY, PointerAction},
    };

    use super::*;
    use crate::{
        abi::{BatonRect, CODEC_H264, KEYBOARD_ELSEWHERE},
        fake_sharer::{Around, AroundPlan, FakeSharer, SharerCall, StartPlan, left_down, left_up},
        platform::Takes,
        session_tests::{QUIET, WAIT},
    };

    /// Quick enough that a test does not wait, slow enough that a loaded machine does not make the
    /// viewer look quiet by accident.
    const TIMING: Timing = Timing {
        poll: Duration::from_millis(20),
        quiet: Duration::from_secs(60),
    };

    /// The thread does not look by itself in the time of a test, and no viewer is ever quiet.
    const LONG: Timing = Timing {
        poll: Duration::from_secs(3600),
        quiet: Duration::from_secs(3600),
    };

    fn geometry() -> Geometry {
        Geometry {
            window: Size {
                width: 400.0,
                height: 300.0,
            },
            picture: Size {
                width: 64.0,
                height: 48.0,
            },
            content: Rect::new(0.0, 0.0, 64.0, 48.0),
        }
    }

    fn at(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    fn pointer(frame: u64, (x, y): (f32, f32), action: PointerAction) -> Event {
        Event::Pointer {
            frame,
            x,
            y,
            modifiers: Modifiers::NONE,
            action,
        }
    }

    fn key(code: u16, down: bool, text: Option<&str>) -> Event {
        Event::Key {
            code,
            down,
            modifiers: Modifiers::NONE,
            repeat: false,
            text: text.map(str::to_owned),
        }
    }

    fn batch(events: Vec<Event>) -> InputBatch {
        InputBatch { events }
    }

    /// What the viewer reports when nothing is the matter.
    fn report() -> Report {
        Report {
            shown: Some(7),
            decode_us: 2_000,
            dropped: 0,
            thermal: Thermal::Nominal,
        }
    }

    impl Input {
        /// How many places in the queue are taken.
        fn queued(&self) -> usize {
            lock(&self.waiting).queue.len()
        }
    }

    /// Waits until `condition` holds.
    fn wait_for(what: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + WAIT;
        while !condition() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(2));
        }
    }

    /// Waits for the thread to end, for as long as `WAIT`: a thread that was not woken when it
    /// should have been fails the test, and does not hang it.
    fn end(thread: JoinHandle<()>) {
        let deadline = Instant::now() + WAIT;
        while !thread.is_finished() {
            if Instant::now() >= deadline {
                // A test that already failed has said what is wrong, and a second panic in a
                // destructor would end the whole run.
                assert!(thread::panicking(), "the thread did not end");
                return;
            }
            thread::sleep(Duration::from_millis(2));
        }
        thread.join().unwrap();
    }

    /// What the thread does next, as `next` says, within `WAIT`: a thread that is not woken when it
    /// should be fails the test and does not hang it. The thread that asked is left to wait.
    fn next_within(input: &Arc<Input>) -> Step {
        let (told, heard) = mpsc::channel();
        let input = Arc::clone(input);
        thread::spawn(move || {
            let _ = told.send(input.next());
        });
        heard
            .recv_timeout(WAIT)
            .expect("the thread was not woken for anything")
    }

    /// The platform stops answering the question about the Mac until it is let go. Dropping it lets
    /// go, so that a test that fails while the thread is stuck does not wait for a thread that
    /// waits for the test.
    struct Stall {
        entered: Arc<AtomicBool>,
        open: Arc<(Mutex<bool>, Condvar)>,
    }

    impl Stall {
        fn new(platform: &FakeSharer) -> Self {
            let entered = Arc::new(AtomicBool::new(false));
            let open = Arc::new((Mutex::new(false), Condvar::new()));
            let (seen, gate) = (Arc::clone(&entered), Arc::clone(&open));
            platform.on_call(move |_, call| {
                if matches!(call, SharerCall::Surroundings) {
                    seen.store(true, Ordering::SeqCst);
                    let (open, changed) = &*gate;
                    let mut open = lock(open);
                    while !*open {
                        open = changed.wait(open).unwrap();
                    }
                }
            });
            Self { entered, open }
        }

        fn wait_until_stuck(&self) {
            wait_for("the thread to be stuck", || {
                self.entered.load(Ordering::SeqCst)
            });
        }

        fn let_go(&self) {
            let (open, changed) = &*self.open;
            *lock(open) = true;
            changed.notify_all();
        }
    }

    impl Drop for Stall {
        fn drop(&mut self) {
            self.let_go();
        }
    }

    /// The rules have a button down, as the thread leaves them after a press.
    fn press(input: &Input) {
        let mut gate = input.gate();
        gate.record_frame(7, Duration::from_millis(10), geometry());
        let context = Context {
            now: Duration::from_millis(20),
            control: true,
            paused: false,
            locked: false,
            on_screen: true,
            window: Rect::new(100.0, 50.0, 400.0, 300.0),
            in_front: &[],
            keyboard: Keyboard::Shared,
        };
        let down = pointer(7, (0.5, 0.5), PointerAction::Down(Button::Left));
        assert!(matches!(gate.handle(&context, &down), Decision::Post(_)));
    }

    /// The window of the fake platform when it has shrunk to 100 by 100 points: the middle of the
    /// old window, at (300, 200), is outside it, and the nearest point inside is (199, 149).
    fn smaller() -> BatonRect {
        BatonRect {
            x: 100.0,
            y: 50.0,
            width: 100.0,
            height: 100.0,
        }
    }

    /// A window of the fake platform served by the thread, and the means to feed it the way the
    /// session does.
    struct Served {
        platform: Arc<FakeSharer>,
        input: Arc<Input>,
        forward: Forward,
        /// What the session was told was dropped.
        dropped: Arc<Mutex<Vec<Reason>>>,
        thread: Option<JoinHandle<()>>,
    }

    impl Served {
        fn new(timing: Timing) -> Self {
            Self::with(timing, |_| {})
        }

        /// As `new`, with the platform set up before the window starts.
        fn with(timing: Timing, setup: impl FnOnce(&Arc<FakeSharer>)) -> Self {
            Self::after(timing, Duration::ZERO, setup)
        }

        /// As `with`, with `delay` between the making of the rules, which is when the person picks
        /// the window, and the start of the window, which is when the other person has said yes.
        fn after(timing: Timing, delay: Duration, setup: impl FnOnce(&Arc<FakeSharer>)) -> Self {
            let platform = FakeSharer::new();
            setup(&platform);
            let mut window = platform
                .platform()
                .adopt(NonNull::new(platform.new_window()).unwrap());
            window
                .prepare(&Takes {
                    max_width: 1920,
                    max_height: 1080,
                    max_fps: 60,
                    codecs: CODEC_H264,
                })
                .unwrap();
            let input = Input::new(timing);
            thread::sleep(delay);
            let dropped = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&dropped);
            let worker = Worker::new(
                Arc::clone(&input),
                window,
                Box::new(move |reason| lock(&sink).push(reason)),
            );
            let (told, started) = mpsc::sync_channel(1);
            let thread = thread::spawn(move || worker.run(1, &told));
            let started = started
                .recv_timeout(WAIT)
                .expect("the thread did not say how the window started");
            Self {
                platform,
                forward: Forward::new(Arc::clone(&input)),
                input,
                dropped,
                thread: started.is_ok().then_some(thread),
            }
        }

        /// The session knows frame 7, which was captured at 10 ms.
        fn knows_a_frame(&self) {
            self.input
                .gate()
                .record_frame(7, Duration::from_millis(10), geometry());
        }

        /// The viewer sent these events, 20 ms into the session.
        fn hears(&mut self, events: Vec<Event>) {
            self.hears_at(events, Duration::from_millis(20));
        }

        /// The viewer sent these events, and they arrived at this time of the session.
        fn hears_at(&mut self, events: Vec<Event>, arrived: Duration) {
            self.forward.input(batch(events), arrived);
        }

        fn posts(&self) -> Vec<Post> {
            self.platform.posts(1)
        }

        fn told(&self) -> Vec<Reason> {
            lock(&self.dropped).clone()
        }

        fn wait_for_posts(&self, count: usize) {
            wait_for("the platform to be told", || self.posts().len() >= count);
        }

        /// Ends the way that the task of the session does: the link is closed and the window is
        /// to be told.
        fn close(&mut self) {
            drop(Closing::new(
                Arc::new(Links::default()),
                1,
                Arc::clone(&self.input),
            ));
            end(self.thread.take().unwrap());
        }
    }

    impl Drop for Served {
        fn drop(&mut self) {
            if let Some(thread) = self.thread.take() {
                self.input.close();
                // A test that failed has said what is wrong, and does not wait for a thread that
                // may be stuck.
                if !thread::panicking() {
                    end(thread);
                }
            }
        }
    }

    #[test]
    fn a_tap_is_put_to_the_window_where_the_rules_put_it() {
        let mut served = Served::new(TIMING);
        served.knows_a_frame();
        // The middle of the picture is the middle of the window, which is at (100, 50) and is 400
        // by 300 points.
        served.hears(vec![
            pointer(7, (0.5, 0.5), PointerAction::Down(Button::Left)),
            pointer(7, (0.5, 0.5), PointerAction::Up(Button::Left)),
        ]);
        served.wait_for_posts(2);
        assert_eq!(
            served.posts(),
            [left_down(300.0, 200.0), left_up(300.0, 200.0)]
        );
        assert_eq!(served.told(), []);
        // The platform was asked what the Mac is doing once for the batch, before the first post,
        // and the window was started with the link it was given.
        let calls = served.platform.calls(1);
        assert_eq!(
            calls[1..],
            [
                SharerCall::Start { link: 1 },
                SharerCall::Surroundings,
                SharerCall::Post(left_down(300.0, 200.0)),
                SharerCall::Post(left_up(300.0, 200.0)),
            ]
        );
        served.platform.assert_no_complaints();
    }

    #[test]
    fn a_batch_asks_the_platform_about_its_surroundings_once_however_many_events_it_has() {
        let mut served = Served::new(TIMING);
        served.knows_a_frame();
        let moves = (0..5_u8)
            .map(|step| {
                let across = f32::from(step) / 10.0;
                pointer(7, (across, 0.5), PointerAction::Move)
            })
            .collect();
        served.hears(moves);
        served.wait_for_posts(5);
        let asked = served
            .platform
            .count(|call| matches!(call, SharerCall::Surroundings));
        assert_eq!(asked, 1);
        // Nothing is held, so the thread has no reason to look again either.
        thread::sleep(QUIET);
        assert_eq!(
            served
                .platform
                .count(|call| matches!(call, SharerCall::Surroundings)),
            1
        );
    }

    #[test]
    fn what_the_rules_drop_is_told_to_the_session_with_the_reason_and_not_posted() {
        let mut served = Served::new(TIMING);
        served.knows_a_frame();
        served.hears(vec![
            // A frame that nobody knows, a key when the keyboard is elsewhere, and a release of
            // something that was not pressed.
            pointer(99, (0.5, 0.5), PointerAction::Down(Button::Left)),
            pointer(7, (0.5, 0.5), PointerAction::Up(Button::Left)),
        ]);
        wait_for("both events to be told", || served.told().len() >= 2);
        assert_eq!(served.told(), [Reason::FrameUnknown, Reason::NotPressed]);
        assert_eq!(served.posts(), []);

        served.platform.set_around(Around {
            keyboard: KEYBOARD_ELSEWHERE,
            ..Around::default()
        });
        served.hears(vec![key(0x04, true, Some("a"))]);
        wait_for("the key to be told", || served.told().len() >= 3);
        assert_eq!(served.told()[2], Reason::NoKeyboard);
        assert_eq!(served.posts(), []);
    }

    #[test]
    fn keys_reach_the_window_with_their_text_and_a_release_follows_the_press() {
        let mut served = Served::new(TIMING);
        served.hears(vec![
            key(0x04, true, Some("a")),
            key(0x04, false, None),
            // Text that came with no key goes down once and is not held.
            key(NO_KEY, true, Some("ok")),
        ]);
        served.wait_for_posts(3);
        let typed = |code, down, text: Option<&str>| Post::Key {
            code,
            down,
            modifiers: Modifiers::NONE,
            repeat: false,
            text: text.map(str::to_owned),
        };
        assert_eq!(
            served.posts(),
            [
                typed(0x04, true, Some("a")),
                typed(0x04, false, None),
                typed(NO_KEY, true, Some("ok")),
            ]
        );
        assert_eq!(served.told(), []);
        assert!(!served.input.gate().is_pressing());
    }

    #[test]
    fn a_surroundings_call_that_fails_drops_the_press_and_never_the_release() {
        let mut served = Served::new(TIMING);
        served.knows_a_frame();
        served.hears(vec![pointer(
            7,
            (0.5, 0.5),
            PointerAction::Down(Button::Left),
        )]);
        served.wait_for_posts(1);

        // The platform cannot say where the window is any more: the press that went down is let
        // go of at once, and what comes next is dropped as for a window that is not there.
        served.platform.plan_around(AroundPlan::Fails(-1));
        served.hears(vec![pointer(7, (0.6, 0.5), PointerAction::Move)]);
        served.wait_for_posts(2);
        assert_eq!(served.posts()[1], left_up(300.0, 200.0));
        assert_eq!(served.told(), [Reason::NotOnScreen]);
        assert!(!served.input.gate().is_pressing());

        served.hears(vec![pointer(
            7,
            (0.5, 0.5),
            PointerAction::Down(Button::Left),
        )]);
        wait_for("the press to be told", || served.told().len() >= 2);
        assert_eq!(served.told()[1], Reason::NotOnScreen);
        assert_eq!(served.posts().len(), 2);
    }

    #[test]
    fn a_platform_that_writes_nothing_gets_the_careful_answer_and_the_mac_counts_as_locked() {
        let mut served = Served::new(TIMING);
        served.knows_a_frame();
        served.platform.plan_around(AroundPlan::WritesNothing);
        served.hears(vec![pointer(
            7,
            (0.5, 0.5),
            PointerAction::Down(Button::Left),
        )]);
        wait_for("the press to be told", || !served.told().is_empty());
        assert_eq!(served.told(), [Reason::Locked]);
        assert_eq!(served.posts(), []);
    }

    #[test]
    fn a_platform_that_claims_more_windows_in_front_than_there_is_room_for_is_not_believed() {
        let mut served = Served::new(TIMING);
        served.knows_a_frame();
        served
            .platform
            .plan_around(AroundPlan::Claims(Around::default(), 129));
        served.hears(vec![pointer(
            7,
            (0.5, 0.5),
            PointerAction::Down(Button::Left),
        )]);
        wait_for("the press to be told", || !served.told().is_empty());
        assert_eq!(served.told(), [Reason::NotOnScreen]);
        assert_eq!(served.posts(), []);
    }

    #[test]
    fn the_windows_in_front_that_the_platform_names_cover_the_window_for_the_rules() {
        let mut served = Served::new(TIMING);
        served.knows_a_frame();
        let covering = crate::abi::BatonFront {
            rect: crate::abi::BatonRect {
                x: 250.0,
                y: 150.0,
                width: 100.0,
                height: 100.0,
            },
            flags: 0,
            reserved: 0,
        };
        served.platform.set_around(Around {
            fronts: vec![covering],
            ..Around::default()
        });
        served.hears(vec![
            // (300, 200) is under the window in front, and (110, 60) is not.
            pointer(7, (0.5, 0.5), PointerAction::Down(Button::Left)),
            pointer(7, (0.0, 0.0), PointerAction::Move),
        ]);
        wait_for("both events to be handled", || {
            served.told().len() + served.posts().len() >= 2
        });
        assert_eq!(served.told(), [Reason::Covered]);
        assert_eq!(
            served.posts(),
            [Post::Pointer {
                at: at(100.0, 50.0),
                modifiers: Modifiers::NONE,
                action: PointerAction::Move,
            }]
        );
    }

    #[test]
    fn a_post_that_the_platform_cannot_do_is_told_as_a_rule_that_is_not_known() {
        let mut served = Served::new(TIMING);
        served.knows_a_frame();
        served.platform.plan_post(-1);
        served.hears(vec![pointer(7, (0.5, 0.5), PointerAction::Move)]);
        wait_for("the move to be told", || !served.told().is_empty());
        assert_eq!(served.told(), [Reason::Other]);
        // The platform was asked, and it said no.
        assert_eq!(served.posts().len(), 1);
    }

    #[test]
    fn what_is_held_is_let_go_of_when_the_mac_locks_with_no_event_to_say_so() {
        let mut served = Served::new(TIMING);
        served.knows_a_frame();
        served.hears(vec![pointer(
            7,
            (0.5, 0.5),
            PointerAction::Down(Button::Left),
        )]);
        served.wait_for_posts(1);
        // The thread looks at what is held down, every so often, and the Mac is as it was.
        wait_for("the thread to look again", || {
            served
                .platform
                .count(|call| matches!(call, SharerCall::Surroundings))
                >= 4
        });
        assert_eq!(served.posts().len(), 1);
        assert!(served.input.gate().is_pressing());

        served.platform.set_around(Around {
            locked: true,
            ..Around::default()
        });
        served.wait_for_posts(2);
        assert_eq!(served.posts()[1], left_up(300.0, 200.0));
        assert!(!served.input.gate().is_pressing());
        assert_eq!(served.told(), []);
    }

    #[test]
    fn what_is_held_is_let_go_of_when_the_window_leaves_the_screen() {
        let mut served = Served::new(TIMING);
        served.knows_a_frame();
        served.hears(vec![
            pointer(7, (0.5, 0.5), PointerAction::Down(Button::Left)),
            key(0x04, true, Some("a")),
        ]);
        served.wait_for_posts(2);
        served.platform.set_around(Around {
            on_screen: false,
            ..Around::default()
        });
        served.wait_for_posts(4);
        let posts = served.posts();
        assert!(posts.contains(&left_up(300.0, 200.0)), "{posts:?}");
        assert!(
            posts.iter().any(|post| matches!(
                post,
                Post::Key {
                    code: 0x04,
                    down: false,
                    ..
                }
            )),
            "{posts:?}"
        );
        assert!(!served.input.gate().is_pressing());
    }

    #[test]
    fn a_batch_that_finds_the_mac_locked_lets_go_of_what_is_held() {
        let mut served = Served::new(LONG);
        served.knows_a_frame();
        served.hears(vec![pointer(
            7,
            (0.5, 0.5),
            PointerAction::Down(Button::Left),
        )]);
        served.wait_for_posts(1);
        served.platform.set_around(Around {
            locked: true,
            ..Around::default()
        });
        served.hears(vec![pointer(7, (0.6, 0.5), PointerAction::Move)]);
        served.wait_for_posts(2);
        assert_eq!(served.posts()[1], left_up(300.0, 200.0));
        assert_eq!(served.told(), [Reason::Locked]);
        // The viewer's own release comes later and finds nothing to release.
        served.hears(vec![pointer(
            7,
            (0.6, 0.5),
            PointerAction::Up(Button::Left),
        )]);
        wait_for("the release to be told", || served.told().len() >= 2);
        assert_eq!(served.told()[1], Reason::NotPressed);
        assert_eq!(served.posts().len(), 2);
    }

    #[test]
    fn a_pause_lets_go_of_what_is_held_at_once_and_drops_what_comes_until_it_goes_on() {
        // Only the pause can be what makes the thread look.
        let mut served = Served::new(LONG);
        served.knows_a_frame();
        served.hears(vec![pointer(
            7,
            (0.5, 0.5),
            PointerAction::Down(Button::Left),
        )]);
        served.wait_for_posts(1);

        served.input.pause();
        served.wait_for_posts(2);
        assert_eq!(served.posts()[1], left_up(300.0, 200.0));

        served.hears(vec![pointer(7, (0.5, 0.5), PointerAction::Move)]);
        wait_for("the move to be told", || !served.told().is_empty());
        assert_eq!(served.told(), [Reason::Paused]);
        assert_eq!(served.posts().len(), 2);

        served.input.resume();
        served.hears(vec![pointer(7, (0.5, 0.5), PointerAction::Move)]);
        served.wait_for_posts(3);
        assert_eq!(served.told(), [Reason::Paused]);
    }

    #[test]
    fn a_viewer_that_goes_quiet_has_what_it_holds_let_go_of_and_one_that_reports_does_not() {
        let quiet = Duration::from_millis(800);
        let mut served = Served::new(Timing {
            poll: Duration::from_millis(10),
            quiet,
        });
        served.knows_a_frame();
        served.forward.report(report());
        served.hears(vec![pointer(
            7,
            (0.5, 0.5),
            PointerAction::Down(Button::Left),
        )]);
        served.wait_for_posts(1);

        // It reports, a few times a second, and the press stays down for longer than the viewer
        // may be quiet.
        let began = Instant::now();
        while began.elapsed() < quiet + quiet / 2 {
            served.forward.report(report());
            thread::sleep(Duration::from_millis(40));
        }
        assert_eq!(served.posts().len(), 1);
        assert!(served.input.gate().is_pressing());

        // Then it says nothing, and the press is let go of once it has been quiet for as long as
        // it may be.
        let silent = Instant::now();
        served.wait_for_posts(2);
        assert!(silent.elapsed() > quiet / 2, "{:?}", silent.elapsed());
        assert_eq!(served.posts()[1], left_up(300.0, 200.0));
        assert!(!served.input.gate().is_pressing());
        assert_eq!(served.told(), []);
    }

    #[test]
    fn a_thread_with_nothing_held_waits_until_it_is_woken_and_one_with_a_press_looks_now_and_then()
    {
        let input = Input::new(TIMING);
        let (told, heard) = mpsc::channel();
        let waiter = {
            let input = Arc::clone(&input);
            thread::spawn(move || told.send(input.next()).unwrap())
        };
        // Far longer than the time between two looks.
        assert!(heard.recv_timeout(TIMING.poll * 10).is_err());
        let wanted = batch(vec![key(4, true, None)]);
        Forward::new(Arc::clone(&input)).input(wanted.clone(), Duration::from_secs(1));
        let Step::Batch(got, arrived, _) = heard.recv_timeout(WAIT).unwrap() else {
            panic!("the thread was woken for something other than the batch");
        };
        assert_eq!((got, arrived), (wanted, Duration::from_secs(1)));
        end(waiter);

        // A nudge wakes a thread with nothing held, for a look that finds nothing to do.
        input.pause();
        assert_eq!(next_within(&input), Step::Look);

        // With something held it comes back by itself, and again.
        press(&input);
        assert_eq!(next_within(&input), Step::Look);
        assert_eq!(next_within(&input), Step::Look);
    }

    #[test]
    fn a_batch_lost_to_a_full_queue_is_made_up_for_after_the_batches_ahead_of_it() {
        let mut served = Served::new(TIMING);
        served.knows_a_frame();
        served.hears(vec![pointer(
            7,
            (0.5, 0.5),
            PointerAction::Down(Button::Left),
        )]);
        served.wait_for_posts(1);

        // The platform stops answering. A press is held, so the thread looks at the Mac within a
        // moment and finds itself in a call that does not return.
        let stall = Stall::new(&served.platform);
        stall.wait_until_stuck();

        // The queue holds QUEUE_BATCHES of them, 5 ms apart so that the rate is no reason to drop
        // any. What comes after is lost: three events in two batches, and the release of the
        // press is one of them.
        for step in 0..QUEUE_BATCHES {
            let arrived = Duration::from_millis(100 + 5 * step as u64);
            served.hears_at(vec![pointer(7, (0.5, 0.5), PointerAction::Move)], arrived);
        }
        assert_eq!(served.input.queued(), QUEUE_BATCHES);
        served.hears_at(
            vec![
                pointer(7, (0.5, 0.5), PointerAction::Move),
                pointer(7, (0.5, 0.5), PointerAction::Up(Button::Left)),
            ],
            Duration::from_secs(2),
        );
        served.hears_at(
            vec![pointer(7, (0.5, 0.5), PointerAction::Move)],
            Duration::from_secs(2),
        );
        assert_eq!(served.input.queued(), QUEUE_BATCHES + 1);

        // The platform answers again. The thread puts what was queued to the window, in its
        // order, and only then lets go of the press, and tells the session about the three.
        stall.let_go();
        wait_for("the loss to be told", || served.told().len() >= 3);
        assert_eq!(served.told(), [Reason::TooFast; 3]);
        assert!(!served.input.gate().is_pressing());
        let posts = served.posts();
        assert_eq!(posts.len(), 1 + QUEUE_BATCHES + 1);
        assert_eq!(posts[0], left_down(300.0, 200.0));
        assert!(
            posts[1..=QUEUE_BATCHES].iter().all(|post| matches!(
                post,
                Post::Pointer {
                    action: PointerAction::Move,
                    ..
                }
            )),
            "the queued moves come before the release"
        );
        assert_eq!(posts[QUEUE_BATCHES + 1], left_up(300.0, 200.0));
        served.platform.assert_no_complaints();
    }

    #[test]
    fn an_event_that_waited_for_a_thread_that_was_held_up_is_judged_at_the_time_it_is_put_to_the_window()
     {
        let mut served = Served::new(TIMING);
        served.knows_a_frame();
        // The platform does not answer for a while: a press is held, so the thread looks at the
        // Mac within a moment and finds itself in a call that does not return.
        served.hears(vec![pointer(
            7,
            (0.5, 0.5),
            PointerAction::Down(Button::Left),
        )]);
        served.wait_for_posts(1);
        let stall = Stall::new(&served.platform);
        stall.wait_until_stuck();

        // A tap arrives, 20 ms into the session, aimed at the picture of 10 ms. The window changes
        // at 30 ms: that is after the picture, and after the tap arrived.
        served.hears(vec![pointer(7, (0.5, 0.5), PointerAction::Move)]);
        served.input.gate().record_change(Duration::from_millis(30));
        // The thread is held up for longer than the picture may be out of date, and then goes on.
        thread::sleep(Duration::from_millis(700));
        stall.let_go();
        wait_for("the tap to be told", || !served.told().is_empty());

        // The tap was put to the window when 700 ms had passed since the change, and not when it
        // arrived, which was before the change: it is dropped.
        assert_eq!(served.told(), [Reason::PictureOutOfDate]);
        assert_eq!(served.posts().len(), 1);
    }

    #[test]
    fn the_queue_is_never_longer_than_its_batches_and_one_more_for_what_is_lost() {
        let input = Input::new(TIMING);
        let mut forward = Forward::new(Arc::clone(&input));
        let mark = |input: &Input| lock(&input.waiting).queue.back().cloned();
        for _ in 0..QUEUE_BATCHES {
            forward.input(batch(vec![key(4, true, None)]), Duration::ZERO);
        }
        assert_eq!(input.queued(), QUEUE_BATCHES);

        // A batch with no events loses nothing, also when it finds the queue full.
        forward.input(batch(vec![]), Duration::ZERO);
        assert_eq!(input.queued(), QUEUE_BATCHES);

        // What is lost is counted on one mark, however many batches it was in.
        forward.input(
            batch(vec![key(4, false, None), key(5, true, None)]),
            Duration::ZERO,
        );
        forward.input(batch(vec![key(5, false, None)]), Duration::ZERO);
        assert_eq!(input.queued(), QUEUE_BATCHES + 1);
        assert_eq!(mark(&input), Some(Queued::Lost(3)));
        // A batch with no events loses nothing.
        forward.input(batch(vec![]), Duration::ZERO);
        assert_eq!(input.queued(), QUEUE_BATCHES + 1);
        assert_eq!(mark(&input), Some(Queued::Lost(3)));

        // The thread takes two. There is room again, for one batch, and the next loss is another
        // mark behind it: what was lost before it stays where it was.
        {
            let mut waiting = lock(&input.waiting);
            waiting.queue.pop_front();
            waiting.queue.pop_front();
        }
        forward.input(batch(vec![key(6, true, None)]), Duration::ZERO);
        assert_eq!(input.queued(), QUEUE_BATCHES);
        forward.input(batch(vec![key(6, false, None)]), Duration::ZERO);
        assert_eq!(input.queued(), QUEUE_BATCHES + 1);
        assert_eq!(mark(&input), Some(Queued::Lost(1)));
        let marks: Vec<Queued> = lock(&input.waiting)
            .queue
            .iter()
            .filter(|queued| matches!(queued, Queued::Lost(_)))
            .cloned()
            .collect();
        assert_eq!(marks, [Queued::Lost(3), Queued::Lost(1)]);

        // A count stops at the largest there is.
        *lock(&input.waiting).queue.back_mut().unwrap() = Queued::Lost(usize::MAX - 1);
        forward.input(batch(vec![key(7, true, None); 3]), Duration::ZERO);
        assert_eq!(mark(&input), Some(Queued::Lost(usize::MAX)));
    }

    #[test]
    fn the_end_of_the_session_lets_go_of_what_is_held_and_stops_what_is_queued_and_the_window_is_told_when_closed()
     {
        // The end is what wakes the thread, and not the time between its looks.
        let mut served = Served::new(LONG);
        served.knows_a_frame();
        served.hears(vec![pointer(
            7,
            (0.5, 0.5),
            PointerAction::Down(Button::Left),
        )]);
        served.wait_for_posts(1);

        // The session is over: what is down is released, and what the viewer sent before it does
        // not reach the window.
        served.forward.ended();
        served.hears(vec![pointer(7, (0.5, 0.5), PointerAction::Move)]);
        served.wait_for_posts(2);
        thread::sleep(QUIET);
        assert_eq!(
            served.posts(),
            [left_down(300.0, 200.0), left_up(300.0, 200.0)]
        );
        assert_eq!(served.input.queued(), 0);
        // The window is told that the session is over only when the link has been closed, and the
        // thread waits for that.
        assert!(
            !served.platform.calls(1).contains(&SharerCall::Ended),
            "{:?}",
            served.platform.calls(1)
        );

        served.close();
        let calls = served.platform.calls(1);
        assert_eq!(
            calls[calls.len() - 2..],
            [SharerCall::Ended, SharerCall::Release]
        );
        served.platform.assert_no_complaints();
    }

    #[test]
    fn a_window_that_is_closed_with_something_held_is_let_go_of_before_it_is_told() {
        let mut served = Served::new(TIMING);
        served.knows_a_frame();
        served.hears(vec![pointer(
            7,
            (0.5, 0.5),
            PointerAction::Down(Button::Left),
        )]);
        served.wait_for_posts(1);
        // The runtime goes away before the end of the session is told.
        served.close();
        let calls = served.platform.calls(1);
        assert_eq!(
            calls[calls.len() - 3..],
            [
                SharerCall::Post(left_up(300.0, 200.0)),
                SharerCall::Ended,
                SharerCall::Release
            ]
        );
    }

    #[test]
    fn a_window_whose_start_fails_is_let_go_of_without_being_told_that_it_ended() {
        let platform = FakeSharer::new();
        platform.plan_start(StartPlan::Fail {
            code: -1,
            text: b"No.".to_vec(),
            claimed_len: 3,
        });
        let mut window = platform
            .platform()
            .adopt(NonNull::new(platform.new_window()).unwrap());
        window
            .prepare(&Takes {
                max_width: 1920,
                max_height: 1080,
                max_fps: 60,
                codecs: CODEC_H264,
            })
            .unwrap();
        let worker = Worker::new(Input::new(TIMING), window, Box::new(|_| {}));
        let (told, started) = mpsc::sync_channel(1);
        let thread = thread::spawn(move || worker.run(1, &told));
        let result = started
            .recv_timeout(WAIT)
            .expect("the thread did not say how the window started");
        assert_eq!(result.unwrap_err().0.as_deref(), Some("No."));
        end(thread);
        let calls = platform.calls(1);
        assert_eq!(
            calls[1..],
            [SharerCall::Start { link: 1 }, SharerCall::Release]
        );
    }

    #[test]
    fn a_start_that_nobody_waits_for_is_as_good_as_one_that_failed() {
        let platform = FakeSharer::new();
        let mut window = platform
            .platform()
            .adopt(NonNull::new(platform.new_window()).unwrap());
        window
            .prepare(&Takes {
                max_width: 1920,
                max_height: 1080,
                max_fps: 60,
                codecs: CODEC_H264,
            })
            .unwrap();
        let worker = Worker::new(Input::new(TIMING), window, Box::new(|_| {}));
        let (told, started) = mpsc::sync_channel(1);
        drop(started);
        // The thread returns on its own, having started the window and let go of it.
        end(thread::spawn(move || worker.run(1, &told)));
        let calls = platform.calls(1);
        assert_eq!(
            calls[1..],
            [SharerCall::Start { link: 1 }, SharerCall::Release]
        );
    }

    #[test]
    fn what_the_thread_does_next_goes_by_these_rules() {
        let mut waiting = Waiting::default();
        assert_eq!(waiting.step(), None);

        // A nudge says to look, once.
        waiting.nudged = true;
        assert_eq!(waiting.step(), Some(Step::Look));
        assert_eq!(waiting.step(), None);

        // What waits is given in the order that it happened, and not twice. A nudge that comes
        // with it is for after.
        let first = batch(vec![key(4, true, None)]);
        let second = batch(vec![key(4, false, None)]);
        let (early, late) = (Instant::now(), Instant::now() + Duration::from_secs(1));
        let (one, two) = (Duration::from_secs(1), Duration::from_secs(2));
        waiting
            .queue
            .push_back(Queued::Batch(first.clone(), one, early));
        waiting.queue.push_back(Queued::Lost(3));
        waiting
            .queue
            .push_back(Queued::Batch(second.clone(), two, late));
        waiting.nudged = true;
        assert_eq!(waiting.step(), Some(Step::Batch(first, one, early)));
        assert_eq!(waiting.step(), Some(Step::Lost(3)));
        assert_eq!(waiting.step(), Some(Step::Batch(second, two, late)));
        assert_eq!(waiting.step(), Some(Step::Look));
        assert_eq!(waiting.step(), None);

        // The end comes once, and takes everything that waits with it.
        waiting
            .queue
            .push_back(Queued::Batch(batch(vec![]), Duration::ZERO, early));
        waiting.queue.push_back(Queued::Lost(1));
        waiting.nudged = true;
        waiting.phase = Phase::Over;
        assert_eq!(waiting.step(), Some(Step::Over));
        assert_eq!(waiting.step(), None);
        assert!(waiting.queue.is_empty());
        assert!(!waiting.nudged);

        // Closing beats all of it, and does not stop.
        waiting.phase = Phase::Closed;
        waiting.queue.push_back(Queued::Lost(1));
        assert_eq!(waiting.step(), Some(Step::Close));
        assert_eq!(waiting.step(), Some(Step::Close));

        // The end of the session does not undo a close that came first, which is what the task
        // does when its runtime goes away.
        let input = Input::new(TIMING);
        input.close();
        input.over();
        assert_eq!(lock(&input.waiting).step(), Some(Step::Close));
    }

    #[test]
    fn a_batch_that_arrives_after_the_end_or_the_close_is_not_kept() {
        let input = Input::new(TIMING);
        let mut forward = Forward::new(Arc::clone(&input));
        forward.input(batch(vec![key(4, true, None)]), Duration::ZERO);
        assert_eq!(input.queued(), 1);
        forward.ended();
        forward.input(batch(vec![key(4, true, None)]), Duration::ZERO);
        assert_eq!(input.queued(), 1);

        let input = Input::new(TIMING);
        input.close();
        Forward::new(Arc::clone(&input)).input(batch(vec![key(4, true, None)]), Duration::ZERO);
        assert_eq!(input.queued(), 0);
    }

    #[test]
    fn the_gate_is_never_held_while_the_platform_is_called() {
        let mut served = Served::new(TIMING);
        served.knows_a_frame();
        // The platform asks for the gate from inside its own call, which is what a picture that
        // is sent at that moment does. A thread that held the gate across the call would wait for
        // itself.
        let input = Arc::clone(&served.input);
        let done = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&done);
        served.platform.on_call(move |_, call| {
            if matches!(call, SharerCall::Post(_)) {
                input.gate().record_change(Duration::from_millis(11));
                flag.store(true, Ordering::SeqCst);
            }
        });
        served.hears(vec![pointer(7, (0.5, 0.5), PointerAction::Move)]);
        wait_for("the post", || done.load(Ordering::SeqCst));
    }

    /// The left button going down in the middle of the picture of frame 7.
    fn middle_press() -> Event {
        pointer(7, (0.5, 0.5), PointerAction::Down(Button::Left))
    }

    #[test]
    fn the_thread_looks_once_a_second_and_a_viewer_may_be_quiet_for_ten() {
        assert_eq!(
            Timing::default(),
            Timing {
                poll: Duration::from_secs(1),
                quiet: Duration::from_secs(10),
            }
        );
    }

    #[test]
    fn a_viewer_has_the_whole_quiet_time_from_the_moment_the_window_starts() {
        // The rules are made when the person picks the window, and the window starts when the
        // other person has said yes, which can be long after.
        let delay = Duration::from_millis(300);
        let served = Served::after(TIMING, delay, |_| {});
        wait_for("the viewer to count as heard", || {
            served.input.silence() < delay
        });
    }

    #[test]
    fn what_is_held_is_let_go_of_inside_the_window_that_the_batch_found() {
        let mut served = Served::new(LONG);
        served.knows_a_frame();
        served.hears(vec![middle_press()]);
        served.wait_for_posts(1);
        // The window has shrunk and the Mac has locked: the next batch finds both, and the press
        // goes up at the nearest point inside the window as it is now.
        served.platform.set_around(Around {
            window: smaller(),
            locked: true,
            ..Around::default()
        });
        served.hears(vec![pointer(7, (0.5, 0.5), PointerAction::Move)]);
        served.wait_for_posts(2);
        assert_eq!(served.posts()[1], left_up(199.0, 149.0));
    }

    #[test]
    fn a_surroundings_call_that_fails_leaves_the_window_where_it_was_last_seen() {
        let mut served = Served::new(LONG);
        served.knows_a_frame();
        served.hears(vec![middle_press()]);
        served.wait_for_posts(1);
        // The window shrinks. The picture is of the larger one, so the move is dropped, and the
        // press is still held.
        served.platform.set_around(Around {
            window: smaller(),
            ..Around::default()
        });
        served.hears(vec![pointer(7, (0.5, 0.5), PointerAction::Move)]);
        wait_for("the move to be told", || !served.told().is_empty());
        assert_eq!(served.told(), [Reason::WindowResized]);
        assert_eq!(served.posts().len(), 1);

        // Then the Mac cannot say anything, and the press is let go of where the window was last
        // seen, not where it never was.
        served.platform.plan_around(AroundPlan::Fails(-1));
        served.hears(vec![pointer(7, (0.5, 0.5), PointerAction::Move)]);
        served.wait_for_posts(2);
        assert_eq!(served.posts()[1], left_up(199.0, 149.0));
        assert_eq!(served.told(), [Reason::WindowResized, Reason::NotOnScreen]);
    }

    #[test]
    fn a_look_lets_go_of_what_is_held_inside_the_window_as_it_is_then() {
        let mut served = Served::new(LONG);
        served.knows_a_frame();
        served.hears(vec![middle_press()]);
        served.wait_for_posts(1);
        served.platform.set_around(Around {
            window: smaller(),
            locked: true,
            ..Around::default()
        });
        served.input.nudge();
        served.wait_for_posts(2);
        assert_eq!(served.posts()[1], left_up(199.0, 149.0));
        assert_eq!(served.told(), []);
        // The Mac is asked once for the batch and once for the look, which lets go where it saw
        // the window and does not ask again.
        let asked = served
            .platform
            .calls(1)
            .iter()
            .filter(|call| matches!(call, SharerCall::Surroundings))
            .count();
        assert_eq!(asked, 2);
    }

    #[test]
    fn a_look_that_finds_the_mac_unable_to_say_lets_go_of_what_is_held() {
        let mut served = Served::new(LONG);
        served.knows_a_frame();
        served.hears(vec![middle_press()]);
        served.wait_for_posts(1);
        served.platform.plan_around(AroundPlan::Fails(-1));
        served.input.nudge();
        served.wait_for_posts(2);
        // Where the window was last seen.
        assert_eq!(served.posts()[1], left_up(300.0, 200.0));
        assert!(!served.input.gate().is_pressing());
    }

    #[test]
    fn the_end_of_the_session_lets_go_of_what_is_held_inside_the_window_as_it_is_then() {
        let mut served = Served::new(LONG);
        served.knows_a_frame();
        served.hears(vec![middle_press()]);
        served.wait_for_posts(1);
        served.platform.set_around(Around {
            window: smaller(),
            ..Around::default()
        });
        served.forward.ended();
        served.wait_for_posts(2);
        assert_eq!(served.posts()[1], left_up(199.0, 149.0));
    }

    #[test]
    fn a_window_that_is_not_on_the_screen_takes_nothing_and_its_batch_lets_go_of_what_is_held() {
        let mut served = Served::new(LONG);
        served.knows_a_frame();
        served.hears(vec![middle_press()]);
        served.wait_for_posts(1);
        served.platform.set_around(Around {
            on_screen: false,
            ..Around::default()
        });
        served.hears(vec![middle_press()]);
        served.wait_for_posts(2);
        assert_eq!(served.posts()[1], left_up(300.0, 200.0));
        assert_eq!(served.told(), [Reason::NotOnScreen]);
        assert_eq!(served.posts().len(), 2);
    }

    #[test]
    fn a_batch_that_is_put_to_the_window_while_paused_lets_go_of_what_is_held() {
        let mut served = Served::new(LONG);
        served.knows_a_frame();
        served.hears(vec![middle_press()]);
        served.wait_for_posts(1);
        // The pause is found by the batch and not by the look that `pause` asks for.
        served.input.paused.store(true, Ordering::SeqCst);
        served.hears(vec![pointer(7, (0.5, 0.5), PointerAction::Move)]);
        served.wait_for_posts(2);
        assert_eq!(served.posts()[1], left_up(300.0, 200.0));
        assert_eq!(served.told(), [Reason::Paused]);
    }

    #[test]
    fn the_viewer_is_told_of_no_more_lost_events_than_the_most() {
        // The number is a promise to the viewer, and the test would follow it wherever it went.
        assert_eq!(MOST_LOST_TOLD, 1024);
        let mut served = Served::new(TIMING);
        served.knows_a_frame();
        // The platform stops answering while the thread has the first batch, and the queue fills.
        let stall = Stall::new(&served.platform);
        let a_move = pointer(7, (0.5, 0.5), PointerAction::Move);
        served.hears_at(vec![a_move.clone()], Duration::from_millis(100));
        stall.wait_until_stuck();
        for step in 0..QUEUE_BATCHES {
            let arrived = Duration::from_millis(200 + 5 * step as u64);
            served.hears_at(vec![a_move.clone()], arrived);
        }
        // What does not fit is more events than the viewer is told about.
        for _ in 0..=MOST_LOST_TOLD / MAX_BATCH {
            served.hears_at(vec![a_move.clone(); MAX_BATCH], Duration::from_secs(3));
        }
        assert_eq!(served.input.queued(), QUEUE_BATCHES + 1);
        stall.let_go();
        wait_for("the loss to be told", || {
            served.told().len() >= MOST_LOST_TOLD
        });
        thread::sleep(QUIET);
        assert_eq!(served.told(), vec![Reason::TooFast; MOST_LOST_TOLD]);
    }
}
