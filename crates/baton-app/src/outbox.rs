// SPDX-License-Identifier: Apache-2.0

//! Sends what waits in the outbox when the device it is for can be reached
//! (`docs/adr/0016-outbox.md`).
//!
//! A link or some text that could not be sent because the device was not there is put in the
//! [`Outbox`] by [`App::send_link`](crate::App::send_link) and
//! [`App::send_text`](crate::App::send_text). The loop in this module looks at the outbox when an
//! item is added, when the list of paired devices changes, when a device takes something that is
//! sent to it at once while other items wait for it, and after a pause in between, and sends each
//! item to the device it is for, oldest first. The pauses start at [`Pace::first`] and double up
//! to [`Pace::longest`], so a device that is away for the night is not dialed every few seconds,
//! and they start short again whenever something happens. Nothing else tells the loop that a
//! device has come back: it finds out by dialing, so a device that comes back is sent what waits
//! within the longest pause, and the time that a dial takes, and not at once, unless the person
//! sends it something first and it takes that.
//!
//! One look takes each device that something waits for on a task of its own, so a device that does
//! not answer the dial does not hold up one that does. A device is sent its items one after the
//! other and left alone for this look at the first that cannot go, so a device that is away is
//! dialed once for each look and not once for each item.
//!
//! What ends an item:
//!
//! - The device took it: [`OutboxEnd::Delivered`].
//! - The device answered and did not take it, for a reason that waiting does not change: it is
//!   dropped, and the person is told why ([`OutboxEnd::Refused`]). A device that is receiving too
//!   fast from this one is the exception: that passes, so the item waits for another look.
//! - A day went by without reaching the device: [`OutboxEnd::Expired`].
//! - The person cancelled it, or unpaired the device. Those are the person's own acts, and nothing
//!   is said of them.
//!
//! An item is delivered at least once and, in two cases, perhaps twice. It is struck out when the
//! device's answer comes, and nothing can make taking an item and hearing about it one step: Baton
//! may close after the device took the item and before the strike-out is written, and the answer
//! may be lost on the way, which reads as a device that cannot be reached. Either way the item goes
//! again as a new send, with a number of its own, so the device cannot tell the repeat.

use std::{future::Future, sync::Arc, time::Duration};

use baton_core::{
    Error as CoreError, Node, Outbox, TrustEvent, TrustStore, Waiting,
    clock::{Clock, SystemClock},
    outbox::LIFETIME,
};
use baton_proto::{
    DeviceId,
    message::{DeclineReason, Item, Outcome},
};
use tokio::{
    sync::{Notify, broadcast, broadcast::error::RecvError},
    task::JoinSet,
    time::sleep,
};
use tokio_util::sync::CancellationToken;

use crate::{
    Error, ErrorKind, Event,
    model::{OutboxEnd, OutboxItem, OutboxSettled},
};

/// How long the loop waits between looks.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pace {
    /// The pause after something happened, and the first after the outbox was empty.
    pub(crate) first: Duration,
    /// The longest the pauses grow to.
    pub(crate) longest: Duration,
}

impl Default for Pace {
    fn default() -> Self {
        Self {
            first: Duration::from_secs(2),
            longest: Duration::from_secs(30),
        }
    }
}

/// What the loop sends with, and asks about the devices that it sends to.
pub(crate) trait Courier: Send + Sync + 'static {
    /// Sends `item` to `to`, and says whether it took it.
    fn send(
        &self,
        to: &DeviceId,
        item: Item,
    ) -> impl Future<Output = Result<Outcome, CoreError>> + Send;

    /// What `to` is called, or `None` if it is not paired any more.
    fn name_of(&self, to: &DeviceId) -> Option<String>;
}

/// The node, which sends, and the list of paired devices, which says who they are.
pub(crate) struct NodeCourier {
    pub(crate) node: Arc<Node>,
    pub(crate) trust: TrustStore,
}

impl Courier for NodeCourier {
    fn send(
        &self,
        to: &DeviceId,
        item: Item,
    ) -> impl Future<Output = Result<Outcome, CoreError>> + Send {
        self.node.send(to, item)
    }

    fn name_of(&self, to: &DeviceId) -> Option<String> {
        self.trust
            .get(to)
            .map(|device| device.display_name().to_owned())
    }
}

/// What the looks at the outbox share.
pub(crate) struct Delivery<C> {
    pub(crate) courier: C,
    pub(crate) outbox: Outbox,
    pub(crate) events: broadcast::Sender<Event>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) pace: Pace,
}

impl Delivery<NodeCourier> {
    /// The looks for the running app: they send with `node`, ask `trust` who the devices are, tell
    /// the screens through `events`, and use the clock of the system and the usual pauses.
    pub(crate) fn for_app(
        node: Arc<Node>,
        trust: TrustStore,
        outbox: Outbox,
        events: broadcast::Sender<Event>,
    ) -> Self {
        Self {
            courier: NodeCourier { node, trust },
            outbox,
            events,
            clock: Arc::new(SystemClock),
            pace: Pace::default(),
        }
    }
}

impl<C: Courier> Delivery<C> {
    /// Looks at the outbox until `stopped`, as the module documentation says. `wake` is notified
    /// when an item is added, and `changes` is the list of paired devices, which says that a
    /// device was added, changed or removed.
    pub(crate) async fn run(
        self,
        wake: Arc<Notify>,
        mut changes: broadcast::Receiver<TrustEvent>,
        stopped: CancellationToken,
    ) {
        let me = Arc::new(self);
        let mut pause = me.pace.first;
        let mut hearing = true;
        loop {
            // Nothing is looked at while nothing waits.
            if me.outbox.is_empty() {
                pause = me.pace.first;
                tokio::select! {
                    () = stopped.cancelled() => return,
                    () = wake.notified() => {}
                }
                continue;
            }
            let moved = tokio::select! {
                () = stopped.cancelled() => return,
                moved = Arc::clone(&me).look(&stopped) => moved,
            };
            if moved {
                pause = me.pace.first;
            }
            if me.outbox.is_empty() {
                continue;
            }
            tokio::select! {
                () = stopped.cancelled() => return,
                () = wake.notified() => pause = me.pace.first,
                // A device that is added, or whose details change, may be one that something
                // waits for. A device that is only heard from changes nothing in the list, so
                // that is not a reason to look.
                change = changes.recv(), if hearing => match change {
                    Ok(_) | Err(RecvError::Lagged(_)) => pause = me.pace.first,
                    Err(RecvError::Closed) => hearing = false,
                },
                () = sleep(pause) => pause = pause.saturating_mul(2).min(me.pace.longest),
            }
        }
    }

    /// One look: drops what is old, then tries each device that something waits for. Whether
    /// anything moved: an item was sent, or dropped, or struck out.
    async fn look(self: Arc<Self>, stopped: &CancellationToken) -> bool {
        let mut moved = false;
        let now = self.clock.now_unix();
        for old in self.kept(move |outbox| outbox.expire(now)).await {
            moved = true;
            self.settle(&old, &old.device_name, OutboxEnd::Expired, expired(&old));
        }
        let mut tasks = JoinSet::new();
        for to in self.outbox.devices() {
            tasks.spawn(Arc::clone(&self).deliver_to(to, stopped.clone()));
        }
        while let Some(done) = tasks.join_next().await {
            moved |= done.unwrap_or(false);
        }
        moved
    }

    /// Sends what waits for `to`, oldest first, up to the first that cannot go. Whether anything
    /// moved.
    async fn deliver_to(self: Arc<Self>, to: DeviceId, stopped: CancellationToken) -> bool {
        let mut moved = false;
        loop {
            let Some(name) = self.courier.name_of(&to) else {
                // Unpaired since it was queued: nothing is sent, and the person knows why.
                return self.forget(&to).await || moved;
            };
            let Some(next) = self.outbox.next_for(&to) else {
                return moved;
            };
            let sent = tokio::select! {
                () = stopped.cancelled() => return moved,
                sent = self.courier.send(&to, next.item.clone()) => sent,
            };
            match sent {
                Ok(Outcome::Accepted) => {
                    let summary = describe(&next.item);
                    self.finish(
                        &next,
                        &name,
                        OutboxEnd::Delivered,
                        format!("Sent {summary} to {name}."),
                    )
                    .await;
                }
                // Too fast for that device: the next look sends it.
                Ok(Outcome::Declined(DeclineReason::RateLimited)) => return moved,
                Ok(Outcome::Declined(reason)) => {
                    let why = Error::declined(&name, what(&next.item), reason);
                    self.finish(&next, &name, OutboxEnd::Refused, why.message().to_owned())
                        .await;
                }
                Err(CoreError::NotPaired(_)) => return self.forget(&to).await || moved,
                // Closing: the item is still there when Baton opens again.
                Err(_) if stopped.is_cancelled() => return moved,
                Err(error) => {
                    let error = Error::reaching(&error, &name);
                    if error.kind() == ErrorKind::Unreachable {
                        return moved;
                    }
                    // Something that waiting does not change, such as a kind that the other
                    // device does not handle.
                    self.finish(&next, &name, OutboxEnd::Refused, error.message().to_owned())
                        .await;
                }
            }
            moved = true;
        }
    }

    /// Strikes out `item`, which has ended as `end`, and tells the person.
    async fn finish(&self, item: &Waiting, name: &str, end: OutboxEnd, message: String) {
        let id = item.id;
        // Struck out by someone else, such as the person, while it was being sent: it was sent all
        // the same, and the person is told so.
        let _ = self.kept(move |outbox| outbox.remove(id)).await;
        self.settle(item, name, end, message);
    }

    /// Strikes out everything for `to`, which is not paired any more, and says that the list
    /// changed. Whether there was anything.
    async fn forget(&self, to: &DeviceId) -> bool {
        let to = *to;
        let gone = self.kept(move |outbox| outbox.remove_for(&to)).await;
        if gone.is_empty() {
            return false;
        }
        let _ = self.events.send(Event::OutboxChanged);
        true
    }

    /// Tells the screens that `item` ended as `end`, and that the list changed.
    fn settle(&self, item: &Waiting, name: &str, end: OutboxEnd, message: String) {
        let _ = self.events.send(Event::OutboxSettled(OutboxSettled {
            device_name: name.to_owned(),
            summary: describe(&item.item),
            end,
            message,
        }));
        let _ = self.events.send(Event::OutboxChanged);
    }

    /// Runs `work` on the list on a thread that may block, since the list is in a file.
    async fn kept<T: Default + Send + 'static>(
        &self,
        work: impl FnOnce(&Outbox) -> T + Send + 'static,
    ) -> T {
        let outbox = self.outbox.clone();
        tokio::task::spawn_blocking(move || work(&outbox))
            .await
            .unwrap_or_default()
    }
}

/// The sentence for an item that nobody took for a day.
fn expired(item: &Waiting) -> String {
    format!(
        "Gave up on sending {} to {}: it could not be reached for a day. Send it again when {} \
         is open.",
        describe(&item.item),
        item.device_name,
        item.device_name,
    )
}

/// What `item` is, as it is told to the person: not what it says.
pub(crate) fn describe(item: &Item) -> String {
    match item {
        Item::Link(link) => {
            let host = url::Url::parse(link.url.trim())
                .ok()
                .and_then(|address| address.host_str().map(str::to_owned));
            match host {
                Some(host) => {
                    // A host is at most 253 characters, and a list is no place for that many.
                    let shown: String = host.chars().take(60).collect();
                    let more = if host.chars().count() > 60 { "…" } else { "" };
                    format!("a link ({shown}{more})")
                }
                None => "a link".to_owned(),
            }
        }
        Item::ClipboardText(text) => {
            let count = text.text.chars().count();
            format!(
                "some text ({count} character{})",
                if count == 1 { "" } else { "s" }
            )
        }
    }
}

/// The word for what `item` is, in "did not take the link".
fn what(item: &Item) -> &'static str {
    match item {
        Item::Link(_) => "link",
        Item::ClipboardText(_) => "text",
    }
}

impl OutboxItem {
    /// An item that waits, with `name` for its device.
    pub(crate) fn of(waiting: &Waiting, name: &str) -> Self {
        Self {
            id: waiting.id,
            device_id: waiting.to.to_string(),
            device_name: name.to_owned(),
            summary: describe(&waiting.item),
            queued_at: waiting.queued_at,
            expires_at: waiting.queued_at.saturating_add(LIFETIME.as_secs()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, VecDeque},
        sync::{
            Mutex,
            atomic::{AtomicU64, Ordering},
        },
    };

    use baton_proto::message::{ClipboardText, Link};

    use super::*;

    const NOW: u64 = 1_800_000_000;

    fn peer(n: u8) -> DeviceId {
        DeviceId::from_bytes([n; DeviceId::LEN])
    }

    fn text(text: &str) -> Item {
        Item::ClipboardText(ClipboardText {
            text: text.to_owned(),
        })
    }

    fn link(url: &str) -> Item {
        Item::Link(Link {
            url: url.to_owned(),
            title: None,
        })
    }

    #[derive(Debug)]
    struct TestClock(AtomicU64);

    impl Clock for TestClock {
        fn now_unix(&self) -> u64 {
            self.0.load(Ordering::Relaxed)
        }

        fn monotonic(&self) -> Duration {
            Duration::ZERO
        }
    }

    /// Answers each device as a test says, and notes what it was asked to send.
    #[derive(Default)]
    struct Script {
        /// What each device is called. A device that is not here is not paired.
        names: Mutex<HashMap<DeviceId, String>>,
        /// The answers to give to a device, one for each try. When they are used up, the device is
        /// not reachable.
        answers: Mutex<HashMap<DeviceId, VecDeque<Result<Outcome, CoreError>>>>,
        /// What was sent to whom, in order, including what was not taken.
        tries: Mutex<Vec<(DeviceId, Item)>>,
        /// When each device was dialed, in the order of the dials.
        dialed: Mutex<Vec<(DeviceId, tokio::time::Instant)>>,
        /// Devices whose dial never ends.
        hanging: Mutex<Vec<DeviceId>>,
        /// How long the dial of a device takes before it answers.
        delays: Mutex<HashMap<DeviceId, Duration>>,
        /// What the next send stops, as it starts: the app closing while it is sent to.
        closes: Mutex<Option<CancellationToken>>,
    }

    impl Script {
        fn pair(&self, n: u8, name: &str) {
            self.names.lock().unwrap().insert(peer(n), name.to_owned());
        }

        fn answer(&self, n: u8, answers: impl IntoIterator<Item = Result<Outcome, CoreError>>) {
            self.answers
                .lock()
                .unwrap()
                .entry(peer(n))
                .or_default()
                .extend(answers);
        }

        fn tries_for(&self, n: u8) -> Vec<Item> {
            self.tries
                .lock()
                .unwrap()
                .iter()
                .filter(|(to, _)| *to == peer(n))
                .map(|(_, item)| item.clone())
                .collect()
        }

        /// How long it was from each dial of device `n` to the next.
        fn pauses_for(&self, n: u8) -> Vec<Duration> {
            let times: Vec<_> = self
                .dialed
                .lock()
                .unwrap()
                .iter()
                .filter(|(to, _)| *to == peer(n))
                .map(|(_, at)| *at)
                .collect();
            times.windows(2).map(|two| two[1] - two[0]).collect()
        }
    }

    impl Courier for Arc<Script> {
        fn send(
            &self,
            to: &DeviceId,
            item: Item,
        ) -> impl Future<Output = Result<Outcome, CoreError>> + Send {
            let script = Arc::clone(self);
            let to = *to;
            async move {
                script.tries.lock().unwrap().push((to, item));
                script
                    .dialed
                    .lock()
                    .unwrap()
                    .push((to, tokio::time::Instant::now()));
                if let Some(app) = script.closes.lock().unwrap().take() {
                    app.cancel();
                }
                if script.hanging.lock().unwrap().contains(&to) {
                    std::future::pending::<()>().await;
                }
                let delay = script.delays.lock().unwrap().get(&to).copied();
                if let Some(delay) = delay {
                    sleep(delay).await;
                }
                script
                    .answers
                    .lock()
                    .unwrap()
                    .get_mut(&to)
                    .and_then(VecDeque::pop_front)
                    .unwrap_or(Err(CoreError::Timeout("dialing the device")))
            }
        }

        fn name_of(&self, to: &DeviceId) -> Option<String> {
            self.names.lock().unwrap().get(to).cloned()
        }
    }

    fn pace() -> Pace {
        Pace {
            first: Duration::from_millis(5),
            longest: Duration::from_millis(40),
        }
    }

    struct Rig {
        script: Arc<Script>,
        outbox: Outbox,
        clock: Arc<TestClock>,
        events: broadcast::Receiver<Event>,
        wake: Arc<Notify>,
        /// The list of paired devices telling the loop that it changed. It is kept so that the loop
        /// does not see it end.
        trust: broadcast::Sender<TrustEvent>,
        stopped: CancellationToken,
        task: tokio::task::JoinHandle<()>,
    }

    impl Rig {
        fn start() -> Self {
            Self::start_with(pace())
        }

        fn start_with(pace: Pace) -> Self {
            Self::start_holding(pace, [])
        }

        /// Starts with `items` waiting already, as after a run that left some, and nobody having
        /// said so to the loop: it is to find them itself.
        fn start_holding(pace: Pace, items: impl IntoIterator<Item = (u8, Item)>) -> Self {
            let script = Arc::new(Script::default());
            script.pair(1, "Pocket Phone");
            script.pair(2, "Kitchen iPad");
            let outbox = Outbox::memory();
            for (n, item) in items {
                let name = script.name_of(&peer(n)).unwrap();
                outbox.add(peer(n), &name, item, NOW).unwrap();
            }
            let clock = Arc::new(TestClock(AtomicU64::new(NOW)));
            let (events, receiver) = broadcast::channel(256);
            let wake = Arc::new(Notify::new());
            let stopped = CancellationToken::new();
            let (trust, changes) = broadcast::channel(8);
            let delivery = Delivery {
                courier: Arc::clone(&script),
                outbox: outbox.clone(),
                events,
                clock: Arc::clone(&clock) as Arc<dyn Clock>,
                pace,
            };
            let task = tokio::spawn(delivery.run(Arc::clone(&wake), changes, stopped.clone()));
            Self {
                script,
                outbox,
                clock,
                events: receiver,
                wake,
                trust,
                stopped,
                task,
            }
        }

        /// Queues `item` for device `n` and wakes the loop, as `App::send` does.
        fn queue(&self, n: u8, item: Item) -> u64 {
            let name = self
                .script
                .name_of(&peer(n))
                .unwrap_or_else(|| "Gone".to_owned());
            let id = self
                .outbox
                .add(peer(n), &name, item, self.clock.now_unix())
                .unwrap();
            self.wake.notify_one();
            id
        }

        /// The next event that `pick` likes, within a limit.
        async fn next<T>(&mut self, mut pick: impl FnMut(Event) -> Option<T>) -> T {
            tokio::time::timeout(Duration::from_secs(20), async {
                loop {
                    let event = self.events.recv().await.expect("the events ended");
                    if let Some(found) = pick(event) {
                        return found;
                    }
                }
            })
            .await
            .expect("the event did not come")
        }

        async fn settled(&mut self) -> OutboxSettled {
            self.next(|event| match event {
                Event::OutboxSettled(settled) => Some(settled),
                _ => None,
            })
            .await
        }

        async fn stop(self) {
            self.stopped.cancel();
            self.task.await.unwrap();
        }
    }

    async fn until(mut done: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(20), async {
            while !done() {
                sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("it did not come about");
    }

    #[tokio::test]
    async fn an_item_for_a_device_that_is_not_reachable_waits_and_is_tried_again() {
        let mut rig = Rig::start();
        rig.queue(1, text("hello"));
        until(|| rig.script.tries_for(1).len() >= 3).await;
        assert_eq!(rig.outbox.list().len(), 1, "it was not given up on");

        // The device is there now.
        rig.script.answer(1, [Ok(Outcome::Accepted)]);
        let settled = rig.settled().await;
        assert_eq!(settled.end, OutboxEnd::Delivered);
        assert_eq!(settled.device_name, "Pocket Phone");
        assert_eq!(settled.summary, "some text (5 characters)");
        assert_eq!(
            settled.message,
            "Sent some text (5 characters) to Pocket Phone."
        );
        assert!(rig.outbox.is_empty());
        // The screens are told that the list changed, right after what became of the item.
        assert!(matches!(rig.events.try_recv(), Ok(Event::OutboxChanged)));
        rig.stop().await;
    }

    #[tokio::test]
    async fn what_waits_for_a_device_is_sent_oldest_first() {
        let mut rig = Rig::start();
        rig.script.answer(
            1,
            [
                Ok(Outcome::Accepted),
                Ok(Outcome::Accepted),
                Ok(Outcome::Accepted),
            ],
        );
        rig.queue(1, text("one"));
        rig.queue(1, link("https://example.com/two"));
        rig.queue(1, text("three"));
        for _ in 0..3 {
            assert_eq!(rig.settled().await.end, OutboxEnd::Delivered);
        }
        assert_eq!(
            rig.script.tries_for(1),
            [text("one"), link("https://example.com/two"), text("three")]
        );
        assert!(rig.outbox.is_empty());
        rig.stop().await;
    }

    #[tokio::test]
    async fn a_device_that_is_away_is_dialed_once_for_each_look_and_not_once_for_each_item() {
        // Four items wait for it, and the pause between looks is longer than the test.
        let rig = Rig::start_holding(
            Pace {
                first: Duration::from_hours(1),
                longest: Duration::from_hours(1),
            },
            (0..4).map(|n| (1, text(&n.to_string()))),
        );
        until(|| !rig.script.tries_for(1).is_empty()).await;
        sleep(Duration::from_millis(100)).await;
        assert_eq!(
            rig.script.tries_for(1),
            [text("0")],
            "the other three did not go"
        );
        assert_eq!(rig.outbox.list().len(), 4);
        rig.stop().await;
    }

    #[tokio::test]
    async fn a_device_that_does_not_answer_does_not_hold_up_another() {
        let mut rig = Rig::start();
        rig.script.hanging.lock().unwrap().push(peer(1));
        rig.script.answer(2, [Ok(Outcome::Accepted)]);
        rig.queue(1, text("for the phone"));
        rig.queue(2, text("for the iPad"));
        let settled = rig.settled().await;
        assert_eq!(settled.device_name, "Kitchen iPad");
        assert_eq!(settled.end, OutboxEnd::Delivered);
        assert_eq!(rig.outbox.list().len(), 1, "the phone's item still waits");
        rig.stop().await;
    }

    #[tokio::test]
    async fn a_device_that_does_not_take_an_item_loses_it_and_is_sent_the_next() {
        let mut rig = Rig::start();
        rig.script.answer(
            1,
            [
                Ok(Outcome::Declined(DeclineReason::Blocked)),
                Ok(Outcome::Accepted),
            ],
        );
        rig.queue(1, link("https://example.com/a"));
        rig.queue(1, text("b"));
        let refused = rig.settled().await;
        assert_eq!(refused.end, OutboxEnd::Refused);
        assert_eq!(
            refused.message,
            "Pocket Phone did not take the link: its settings for this device do not allow this \
             kind of item."
        );
        assert_eq!(rig.settled().await.end, OutboxEnd::Delivered);
        assert!(rig.outbox.is_empty());
        rig.stop().await;
    }

    #[tokio::test]
    async fn a_device_that_receives_too_fast_is_tried_again_and_keeps_the_item() {
        let mut rig = Rig::start();
        rig.script.answer(
            1,
            [
                Ok(Outcome::Declined(DeclineReason::RateLimited)),
                Ok(Outcome::Accepted),
            ],
        );
        rig.queue(1, text("hello"));
        assert_eq!(rig.settled().await.end, OutboxEnd::Delivered);
        assert_eq!(rig.script.tries_for(1).len(), 2);
        rig.stop().await;
    }

    #[tokio::test]
    async fn an_error_that_waiting_does_not_change_drops_the_item() {
        let mut rig = Rig::start();
        rig.script
            .answer(1, [Err(CoreError::Unsupported("links".to_owned()))]);
        rig.queue(1, link("https://example.com/"));
        let refused = rig.settled().await;
        assert_eq!(refused.end, OutboxEnd::Refused);
        assert!(
            refused
                .message
                .contains("does not handle this kind of item"),
            "{}",
            refused.message
        );
        assert!(rig.outbox.is_empty());
        rig.stop().await;
    }

    #[tokio::test]
    async fn an_item_that_waited_a_day_is_dropped_and_the_person_is_told() {
        let mut rig = Rig::start();
        rig.queue(1, text("old"));
        until(|| !rig.script.tries_for(1).is_empty()).await;
        // Mid-way: still waiting.
        rig.clock
            .0
            .store(NOW + LIFETIME.as_secs() - 1, Ordering::Relaxed);
        sleep(Duration::from_millis(100)).await;
        assert_eq!(rig.outbox.list().len(), 1);

        rig.clock
            .0
            .store(NOW + LIFETIME.as_secs(), Ordering::Relaxed);
        let expired = rig.settled().await;
        assert_eq!(expired.end, OutboxEnd::Expired);
        assert_eq!(
            expired.message,
            "Gave up on sending some text (3 characters) to Pocket Phone: it could not be reached \
             for a day. Send it again when Pocket Phone is open."
        );
        assert!(rig.outbox.is_empty());
        rig.stop().await;
    }

    #[tokio::test]
    async fn what_waits_for_a_device_that_is_not_paired_any_more_is_dropped_without_a_word() {
        let mut rig = Rig::start();
        rig.script.names.lock().unwrap().remove(&peer(1));
        rig.queue(1, text("hello"));
        rig.next(|event| matches!(event, Event::OutboxChanged).then_some(()))
            .await;
        assert!(rig.outbox.is_empty());
        assert!(rig.script.tries_for(1).is_empty(), "nothing was sent to it");
        // And nothing was said of it.
        while let Ok(event) = rig.events.try_recv() {
            assert!(!matches!(event, Event::OutboxSettled(_)), "{event:?}");
        }
        rig.stop().await;
    }

    #[tokio::test]
    async fn a_new_item_is_tried_at_once_and_not_after_the_pause() {
        let mut rig = Rig::start_with(Pace {
            first: Duration::from_hours(1),
            longest: Duration::from_hours(1),
        });
        // Nothing waits: nothing is looked at, however long the pause.
        sleep(Duration::from_millis(50)).await;
        assert!(rig.script.tries.lock().unwrap().is_empty());
        rig.script.answer(1, [Ok(Outcome::Accepted)]);
        rig.queue(1, text("now"));
        assert_eq!(rig.settled().await.end, OutboxEnd::Delivered);
        rig.stop().await;
    }

    #[tokio::test]
    async fn a_change_in_the_list_of_paired_devices_is_a_reason_to_look_at_once() {
        let mut rig = Rig::start_holding(
            Pace {
                first: Duration::from_hours(1),
                longest: Duration::from_hours(1),
            },
            [(1, text("hello"))],
        );
        until(|| !rig.script.tries_for(1).is_empty()).await;
        sleep(Duration::from_millis(100)).await;
        assert_eq!(rig.script.tries_for(1).len(), 1, "the pause is an hour");

        // The device is back, and its details changed when it connected, which the list of paired
        // devices says: that is not waited out.
        rig.script.answer(1, [Ok(Outcome::Accepted)]);
        rig.trust.send(TrustEvent::Updated(peer(1))).unwrap();
        assert_eq!(rig.settled().await.end, OutboxEnd::Delivered);
        rig.stop().await;
    }

    #[tokio::test]
    async fn what_an_earlier_run_left_is_tried_without_anybody_saying_so() {
        let mut rig = Rig::start_holding(pace(), [(1, text("left over"))]);
        rig.script.answer(1, [Ok(Outcome::Accepted)]);
        let settled = rig.settled().await;
        assert_eq!(settled.end, OutboxEnd::Delivered);
        assert_eq!(rig.script.tries_for(1), [text("left over")]);
        rig.stop().await;
    }

    #[tokio::test]
    async fn stopping_ends_the_loop_and_keeps_what_waits() {
        let rig = Rig::start();
        rig.script.hanging.lock().unwrap().push(peer(1));
        rig.queue(1, text("hello"));
        until(|| !rig.script.tries_for(1).is_empty()).await;
        let outbox = rig.outbox.clone();
        // The send that never ends is dropped, and the loop with it.
        rig.stop().await;
        assert_eq!(outbox.list().len(), 1, "it is there when Baton opens again");
    }

    #[tokio::test]
    async fn a_device_that_is_unpaired_as_it_is_dialed_loses_what_waited_for_it_without_a_word() {
        let mut rig = Rig::start();
        // It is paired when the loop looks, and is not by the time that it answers.
        rig.script.answer(1, [Err(CoreError::NotPaired(peer(1)))]);
        rig.queue(1, text("hello"));
        rig.next(|event| matches!(event, Event::OutboxChanged).then_some(()))
            .await;
        assert!(rig.outbox.is_empty());
        assert_eq!(rig.script.tries_for(1).len(), 1);
        while let Ok(event) = rig.events.try_recv() {
            assert!(!matches!(event, Event::OutboxSettled(_)), "{event:?}");
        }
        rig.stop().await;
    }

    #[tokio::test]
    async fn closing_as_an_item_is_sent_keeps_it_whatever_the_send_says() {
        let rig = Rig::start();
        // Baton closes as the send starts, and the send ends with something that waiting does not
        // change, which would drop the item at any other time.
        *rig.script.closes.lock().unwrap() = Some(rig.stopped.clone());
        rig.script
            .answer(1, [Err(CoreError::Unsupported("links".to_owned()))]);
        rig.queue(1, link("https://example.com/"));
        until(|| !rig.script.tries_for(1).is_empty()).await;
        let outbox = rig.outbox.clone();
        rig.stop().await;
        // Whatever was going to be struck out is struck out by a thread of its own.
        sleep(Duration::from_millis(100)).await;
        assert_eq!(outbox.list().len(), 1, "it is there when Baton opens again");
    }

    #[tokio::test]
    async fn forgetting_what_is_not_there_changes_nothing_and_says_nothing() {
        let script = Arc::new(Script::default());
        let outbox = Outbox::memory();
        let (events, mut seen) = broadcast::channel(8);
        let delivery = Delivery {
            courier: Arc::clone(&script),
            outbox: outbox.clone(),
            events,
            clock: Arc::new(TestClock(AtomicU64::new(NOW))),
            pace: pace(),
        };
        assert!(!delivery.forget(&peer(1)).await);
        assert!(seen.try_recv().is_err());

        outbox.add(peer(1), "A", text("a"), NOW).unwrap();
        outbox.add(peer(2), "B", text("b"), NOW).unwrap();
        assert!(delivery.forget(&peer(1)).await);
        assert_eq!(
            outbox.list().len(),
            1,
            "what waits for another device stays"
        );
        assert!(matches!(seen.try_recv(), Ok(Event::OutboxChanged)));
    }

    /// The pace for the tests that count seconds. They take no time at all: while the clock of
    /// the runtime is stopped (`start_paused`), it jumps to the next thing that waits for it
    /// whenever nothing else can happen, and the times that the courier notes are exact.
    fn slow_pace() -> Pace {
        Pace {
            first: Duration::from_secs(2),
            longest: Duration::from_secs(30),
        }
    }

    fn secs(pauses: &[Duration]) -> Vec<u64> {
        pauses.iter().map(Duration::as_secs).collect()
    }

    #[tokio::test(start_paused = true)]
    async fn the_pauses_between_looks_double_up_to_the_longest_and_no_further() {
        let rig = Rig::start_holding(slow_pace(), [(1, text("hello"))]);
        // The looks are at 0, 2, 6, 14, 30, 60, 90, 120, 150 and 180 seconds.
        sleep(Duration::from_secs(200)).await;
        assert_eq!(
            secs(&rig.script.pauses_for(1)),
            [2, 4, 8, 16, 30, 30, 30, 30, 30]
        );
        rig.stop().await;
    }

    #[tokio::test(start_paused = true)]
    async fn an_item_that_is_added_makes_the_pauses_start_again_at_the_first() {
        let rig = Rig::start_holding(slow_pace(), [(1, text("for the phone"))]);
        // The pause has grown to the longest: the looks were at 0, 2, 6, 14 and 30 seconds.
        sleep(Duration::from_secs(31)).await;
        // The new item is tried at once, and the next look is two seconds later, not thirty.
        rig.queue(2, text("for the iPad"));
        sleep(Duration::from_secs(10)).await;
        assert_eq!(secs(&rig.script.pauses_for(1)), [2, 4, 8, 16, 1, 2, 4]);
        assert_eq!(secs(&rig.script.pauses_for(2)), [2, 4]);
        rig.stop().await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_change_to_the_paired_devices_makes_the_pauses_start_again_at_the_first() {
        let rig = Rig::start_holding(slow_pace(), [(1, text("for the phone"))]);
        sleep(Duration::from_secs(31)).await;
        rig.trust.send(TrustEvent::Updated(peer(1))).unwrap();
        sleep(Duration::from_secs(10)).await;
        assert_eq!(secs(&rig.script.pauses_for(1)), [2, 4, 8, 16, 1, 2, 4]);
        rig.stop().await;
    }

    #[tokio::test(start_paused = true)]
    async fn an_item_that_goes_makes_the_pauses_start_again_at_the_first() {
        let rig = Rig::start_holding(
            slow_pace(),
            [(1, text("for the phone")), (2, text("for the iPad"))],
        );
        sleep(Duration::from_secs(31)).await;
        // The phone is back, and nobody says so: the look at 60 seconds finds it. That is something
        // moving, so the iPad, which is still away, is tried again after two seconds and not thirty.
        rig.script.answer(1, [Ok(Outcome::Accepted)]);
        sleep(Duration::from_secs(40)).await;
        assert_eq!(rig.script.tries_for(1).len(), 6);
        assert_eq!(secs(&rig.script.pauses_for(2)), [2, 4, 8, 16, 30, 2, 4]);
        rig.stop().await;
    }

    #[tokio::test(start_paused = true)]
    async fn an_item_that_goes_counts_as_progress_whichever_device_is_done_last() {
        let rig = Rig::start_holding(
            slow_pace(),
            [(1, text("for the phone")), (2, text("for the iPad"))],
        );
        // The iPad takes a second to say that it is not there, so it is the last to be done with.
        rig.script
            .delays
            .lock()
            .unwrap()
            .insert(peer(2), Duration::from_secs(1));
        sleep(Duration::from_secs(40)).await;
        // The phone is back, and the look at 65 seconds finds it. The iPad, which says no again
        // after the phone has gone, does not undo that: the pauses start again at the first. A
        // look takes a second here, so each time between two dials is a second more than the pause.
        rig.script.answer(1, [Ok(Outcome::Accepted)]);
        sleep(Duration::from_secs(40)).await;
        assert_eq!(rig.script.tries_for(1).len(), 6);
        assert_eq!(secs(&rig.script.pauses_for(2)), [3, 5, 9, 17, 31, 3, 5]);
        rig.stop().await;
    }

    #[tokio::test(start_paused = true)]
    async fn an_item_that_comes_when_nothing_waited_is_tried_with_pauses_that_start_at_the_first() {
        let rig = Rig::start_holding(slow_pace(), [(1, text("one"))]);
        // The pause has grown to the longest: the looks were at 0, 2, 6, 14 and 30 seconds.
        sleep(Duration::from_secs(31)).await;
        // The person gives up on it, and the loop finds nothing to look at, and waits to be told.
        assert!(rig.outbox.remove(1).is_some());
        sleep(Duration::from_secs(100)).await;
        // Another item comes: it is tried at once, and then after 2, 4 and 8 seconds.
        rig.queue(1, text("two"));
        sleep(Duration::from_secs(20)).await;
        assert_eq!(secs(&rig.script.pauses_for(1)), [2, 4, 8, 16, 101, 2, 4, 8]);
        rig.stop().await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_loop_with_nothing_to_send_looks_at_nothing_until_it_is_told() {
        let mut rig = Rig::start_holding(slow_pace(), [(1, text("only"))]);
        rig.script.answer(1, [Ok(Outcome::Accepted)]);
        assert_eq!(rig.settled().await.end, OutboxEnd::Delivered);
        // Everything that could happen has: the loop has nothing to wait for but to be told.
        sleep(Duration::from_millis(1)).await;

        // An item is put in the list and nobody says so. The loop does not look for it.
        rig.outbox
            .add(peer(1), "Pocket Phone", text("quiet"), NOW)
            .unwrap();
        rig.script.answer(1, [Ok(Outcome::Accepted)]);
        sleep(Duration::from_hours(1)).await;
        assert_eq!(rig.script.tries_for(1), [text("only")]);

        rig.wake.notify_one();
        assert_eq!(rig.settled().await.end, OutboxEnd::Delivered);
        assert_eq!(rig.script.tries_for(1), [text("only"), text("quiet")]);
        rig.stop().await;
    }

    #[tokio::test]
    async fn a_list_of_devices_that_ends_does_not_make_the_loop_look_over_and_over() {
        let rig = Rig::start();
        rig.queue(1, text("hello"));
        // The list of devices is gone, as the loop sees it end.
        drop(rig.trust);
        sleep(Duration::from_millis(300)).await;
        // The pauses are 5, 10, 20 and then 40 milliseconds, so about ten looks. A loop that took
        // the end of the list for something to look at would look as often as it could.
        let dials = rig.script.tries_for(1).len();
        assert!(dials < 30, "{dials} looks in a third of a second");
        rig.stopped.cancel();
        rig.task.await.unwrap();
    }

    #[test]
    fn an_item_is_told_by_what_it_is_and_not_by_what_it_says() {
        assert_eq!(
            describe(&link("https://example.com/path?token=hunter2#frag")),
            "a link (example.com)"
        );
        assert_eq!(describe(&link("not an address")), "a link");
        assert_eq!(describe(&text("x")), "some text (1 character)");
        assert_eq!(describe(&text("héllo")), "some text (5 characters)");
        // A host is shown to sixty characters, and a longer one says that it was cut.
        let host = |length: usize| link(&format!("https://{}/", "a".repeat(length)));
        assert_eq!(describe(&host(60)), format!("a link ({})", "a".repeat(60)));
        assert_eq!(describe(&host(61)), format!("a link ({}…)", "a".repeat(60)));
        assert_eq!(
            describe(&host(200)),
            format!("a link ({}…)", "a".repeat(60))
        );
        assert!(!describe(&text("hunter2")).contains("hunter2"));
    }

    #[test]
    fn an_item_of_the_list_says_when_it_is_given_up_on() {
        let waiting = Waiting {
            id: 7,
            to: peer(1),
            device_name: "Old name".to_owned(),
            queued_at: NOW,
            item: text("hello"),
        };
        let item = OutboxItem::of(&waiting, "Pocket Phone");
        assert_eq!(item.id, 7);
        assert_eq!(item.device_name, "Pocket Phone");
        assert_eq!(item.queued_at, NOW);
        assert_eq!(item.expires_at, NOW + 24 * 60 * 60);
        assert_eq!(item.device_id, peer(1).to_string());
    }
}
