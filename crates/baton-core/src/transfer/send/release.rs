// SPDX-License-Identifier: Apache-2.0

//! Telling receivers to let go of the transfers that this device left behind.
//!
//! [`run`] is a task of a node. It asks [`Holds`] which transfers no transfer of this run is going on
//! with, connects to each device that may hold some (all at once, so that one that cannot be reached
//! does not keep the others waiting), and sends `Cancel` for each one (section 8, Cancel). A receiver
//! that answers has let go, or never had the transfer, and the transfer is struck out. One that
//! cannot be reached, or does not answer, is tried again after a pause that doubles up to a few
//! minutes, for as long as [`Holds`] keeps the transfer. A receiver counts the hour that it holds a
//! transfer on a clock that does not run while it sleeps, so a transfer is not given up for being
//! old: once the last word about every transfer that is left is more than an hour old, only a
//! receiver that slept can still hold one, and the pauses grow to half an hour.
//!
//! It starts with the node, so what an earlier run left behind is given up as soon as the device
//! that holds it is there, and a transfer that is left while the node runs wakes it at once.
//!
//! A device is told through the connections that the node makes for a transfer, and `Cancel` goes to
//! any paired device, whatever its hello says about taking files: a device that stopped taking files
//! may hold a transfer that it took before. A device that turns this one away cannot be told
//! anything, and what it holds it lets go of by itself in the hour that the receiver keeps it.

use std::{sync::Arc, time::Duration};

use baton_proto::{DeviceId, id::TransferId};
use tokio::{task::JoinSet, time::sleep};
use tokio_util::sync::CancellationToken;

use super::{
    holds::Holds,
    wire::{self, Wire, WireError},
};
use crate::clock::Clock;

/// How the release paces itself.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pace {
    /// The pause after a look that left something to be done. It doubles with each look that does.
    pub(crate) start: Duration,
    /// The longest pause while a transfer was left lately (see [`Holds::recent`]).
    pub(crate) max: Duration,
    /// The longest pause when none was.
    pub(crate) late: Duration,
    /// The longest each `Cancel` is waited for.
    pub(crate) wait: Duration,
}

impl Default for Pace {
    fn default() -> Self {
        Self {
            start: Duration::from_secs(1),
            max: Duration::from_mins(5),
            late: Duration::from_mins(30),
            wait: Duration::from_secs(10),
        }
    }
}

/// Gives back the transfers in `holds` until the node stops. `wires` makes the connections to a
/// device, and `paired` says whether it is one that can still be told.
pub(crate) async fn run(
    holds: Holds,
    wires: impl Fn(DeviceId) -> Arc<dyn Wire> + Send,
    paired: Arc<dyn Fn(&DeviceId) -> bool + Send + Sync>,
    clock: Arc<dyn Clock>,
    pace: Pace,
    stop: CancellationToken,
) {
    let mut left = holds.watch_left();
    let mut pause = pace.start;
    loop {
        // A transfer that is left from here on is seen below or woken for, so it is marked as seen
        // before the look.
        drop(left.borrow_and_update());
        let due = {
            let (paired, now) = (Arc::clone(&paired), clock.now_unix());
            holds
                .with(move |holds| holds.loose(&*paired, now))
                .await
                .unwrap_or_default()
        };
        if due.is_empty() {
            pause = pace.start;
            tokio::select! {
                biased;
                () = stop.cancelled() => return,
                _ = left.changed() => {}
            }
            continue;
        }
        // Each device on its own, so that one that cannot be reached for twenty seconds does not keep
        // the others waiting.
        let mut tasks = JoinSet::new();
        for (peer, ids) in due {
            let (holds, wire, stop) = (holds.clone(), wires(peer), stop.clone());
            tasks.spawn(async move { release(&holds, &*wire, peer, ids, &pace, &stop).await });
        }
        let mut every = true;
        while let Some(done) = tasks.join_next().await {
            // A task that did not come back has told nobody anything.
            every &= done.unwrap_or(false);
        }
        if every {
            pause = pace.start;
            continue;
        }
        // What is left is tried again after a pause, or at once when another transfer is left, for
        // which the device may be there. The pauses grow longer when nothing was left lately.
        let now = clock.now_unix();
        let recent = holds
            .with(move |holds| holds.recent(now))
            .await
            .unwrap_or(true);
        let most = if recent { pace.max } else { pace.late };
        tokio::select! {
            biased;
            () = stop.cancelled() => return,
            _ = left.changed() => pause = pace.start,
            () = sleep(pause) => pause = pause.saturating_mul(2).min(most),
        }
    }
}

/// Tells `peer` to let go of each of the transfers `ids`, and strikes out each that does. Says
/// whether it was done for all of them: `false` for a device that could not be reached or did not
/// say so, and for one that was told to stop, and what is left is asked again later.
async fn release(
    holds: &Holds,
    wire: &dyn Wire,
    peer: DeviceId,
    ids: Vec<TransferId>,
    pace: &Pace,
    stop: &CancellationToken,
) -> bool {
    let connected = tokio::select! {
        biased;
        () = stop.cancelled() => return false,
        connected = wire.connect() => connected,
    };
    let link = match connected {
        Ok(link) => link,
        Err(WireError::Refused(_)) => {
            holds.with(move |holds| holds.forget(&peer)).await;
            return true;
        }
        Err(WireError::Lost | WireError::TimedOut) => return false,
    };
    let mut every = true;
    for id in ids {
        let told = tokio::select! {
            biased;
            () = stop.cancelled() => return false,
            told = wire::cancel(&*link, id, pace.wait) => told,
        };
        match told {
            Ok(true) => {
                holds.with(move |holds| holds.released(&peer, id)).await;
            }
            // An answer that is not about the transfer is not the receiver letting go of it.
            Ok(false) => every = false,
            Err(WireError::Refused(_)) => {
                holds.with(move |holds| holds.forget(&peer)).await;
                return true;
            }
            // The connection is gone, and the rest would meet the same.
            Err(WireError::Lost | WireError::TimedOut) => return false,
        }
    }
    every
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicBool, Ordering},
        time::Instant,
    };

    use baton_proto::{
        message::{DeclineReason, ErrorCode},
        transfer::TransferRequest,
    };

    use super::*;
    use crate::{
        clock::ManualClock,
        transfer::send::{
            holds::HELD_FOR,
            testing::{
                Call, Connect, Event, Kind, Script, Step, cancelled, declined, error, id, until,
                within,
            },
        },
        util::{BoxFuture, lock},
    };

    const NOW: u64 = 1_000_000;

    /// A pace that a test can sit through.
    fn quick() -> Pace {
        Pace {
            start: Duration::from_millis(20),
            max: Duration::from_millis(80),
            late: Duration::from_millis(320),
            wait: Duration::from_millis(200),
        }
    }

    fn peer(n: u8) -> DeviceId {
        DeviceId::from_bytes([n; DeviceId::LEN])
    }

    /// Holds that are left: `begin` and `ended` for each of `transfers` of `device`.
    fn left(holds: &Holds, device: u8, transfers: &[u8]) {
        for &transfer in transfers {
            holds.begin(peer(device), id(transfer), NOW);
            holds.ended(&peer(device), id(transfer), false, NOW);
        }
    }

    /// What `Cancel` is answered with by a receiver that gives up what it is asked about: the nth
    /// request of a connection is about the nth of `transfers`.
    fn gives_up(transfers: &'static [u8]) -> impl Fn(&Call) -> Step {
        move |call| {
            assert_eq!(call.kind, Kind::Cancel);
            cancelled(id(transfers[call.nth]))
        }
    }

    /// The node's side of the release: a wire for each device, from `scripts`.
    struct Release {
        holds: Holds,
        stop: CancellationToken,
        task: tokio::task::JoinHandle<()>,
    }

    impl Release {
        fn start(
            holds: &Holds,
            scripts: Vec<(DeviceId, Arc<Script>)>,
            paired: impl Fn(&DeviceId) -> bool + Send + Sync + 'static,
            now: u64,
        ) -> Self {
            Self::paced(holds, scripts, paired, now, quick())
        }

        fn paced(
            holds: &Holds,
            scripts: Vec<(DeviceId, Arc<Script>)>,
            paired: impl Fn(&DeviceId) -> bool + Send + Sync + 'static,
            now: u64,
            pace: Pace,
        ) -> Self {
            let stop = CancellationToken::new();
            let wires = move |device: DeviceId| -> Arc<dyn Wire> {
                let (_, script) = scripts
                    .iter()
                    .find(|(owner, _)| *owner == device)
                    .expect("a wire for the device");
                Arc::new(Shared(Arc::clone(script)))
            };
            let task = tokio::spawn(run(
                holds.clone(),
                wires,
                Arc::new(paired),
                Arc::new(ManualClock::new(now)),
                pace,
                stop.clone(),
            ));
            Self {
                holds: holds.clone(),
                stop,
                task,
            }
        }

        /// Stops the release, and waits for it to have stopped.
        async fn stop(self) {
            self.stop.cancel();
            within("the release to stop", self.task).await.unwrap();
        }

        async fn until_nothing_is_held(&self) {
            until("the transfers to be given up", || {
                self.holds.all().is_empty()
            })
            .await;
        }
    }

    /// A script that several tasks hold, as a node's wire is held by the task of the release.
    struct Shared(Arc<Script>);

    impl Wire for Shared {
        fn connect(&self) -> BoxFuture<'_, Result<Box<dyn wire::Link>, WireError>> {
            self.0.connect()
        }
    }

    fn cancels_of(script: &Script) -> Vec<TransferId> {
        script
            .requests()
            .into_iter()
            .map(|request| match request {
                TransferRequest::Cancel(cancel) => cancel.id,
                other => panic!("a request that is not a cancel: {other:?}"),
            })
            .collect()
    }

    fn connections(script: &Script) -> usize {
        script
            .log()
            .events()
            .iter()
            .filter(|event| matches!(event, Event::Connect(_)))
            .count()
    }

    #[test]
    fn the_pauses_start_at_a_second_and_grow_to_five_minutes_or_half_an_hour() {
        // The numbers that the documentation gives: protocol.md and architecture.md.
        let pace = Pace::default();

        assert_eq!(pace.start, Duration::from_secs(1));
        assert_eq!(pace.max, Duration::from_mins(5));
        assert_eq!(pace.late, Duration::from_mins(30));
        assert_eq!(pace.wait, Duration::from_secs(10));
    }

    #[tokio::test]
    async fn the_transfers_that_were_left_are_given_up_on_one_connection_and_struck_out() {
        let holds = Holds::memory();
        left(&holds, 1, &[1, 2]);
        let script = Arc::new(Script::new(gives_up(&[1, 2])));

        let release = Release::start(&holds, vec![(peer(1), Arc::clone(&script))], |_| true, NOW);
        release.until_nothing_is_held().await;

        assert_eq!(cancels_of(&script), [id(1), id(2)]);
        assert_eq!(connections(&script), 1);
        release.stop().await;
    }

    #[tokio::test]
    async fn each_device_is_told_on_a_connection_of_its_own() {
        let holds = Holds::memory();
        left(&holds, 1, &[1]);
        left(&holds, 2, &[2]);
        let first = Arc::new(Script::new(gives_up(&[1])));
        let second = Arc::new(Script::new(gives_up(&[2])));

        let release = Release::start(
            &holds,
            vec![
                (peer(1), Arc::clone(&first)),
                (peer(2), Arc::clone(&second)),
            ],
            |_| true,
            NOW,
        );
        release.until_nothing_is_held().await;

        assert_eq!(cancels_of(&first), [id(1)]);
        assert_eq!(cancels_of(&second), [id(2)]);
        release.stop().await;
    }

    #[tokio::test]
    async fn a_receiver_that_takes_no_part_in_transfers_has_nothing_of_it() {
        let holds = Holds::memory();
        left(&holds, 1, &[1]);
        let script = Arc::new(Script::new(|_| declined(id(1), DeclineReason::Unsupported)));

        let release = Release::start(&holds, vec![(peer(1), script)], |_| true, NOW);

        release.until_nothing_is_held().await;
        release.stop().await;
    }

    #[tokio::test]
    async fn a_receiver_that_cannot_be_reached_is_tried_again_after_longer_and_longer_pauses() {
        let holds = Holds::memory();
        left(&holds, 1, &[1]);
        let script = Arc::new(
            Script::new(gives_up(&[1]))
                .connects(|n| if n < 4 { Connect::Lost } else { Connect::Ok }),
        );

        let release = Release::start(&holds, vec![(peer(1), Arc::clone(&script))], |_| true, NOW);
        release.until_nothing_is_held().await;

        // Four connections, three of them failed, with a pause before each of the next three that
        // is at least what the pace says: 20 ms, then 40 ms, then 80 ms, which is the most.
        let times: Vec<Instant> = script
            .log()
            .times()
            .into_iter()
            .filter(|(_, event)| matches!(event, Event::Connect(_) | Event::ConnectFailed(_)))
            .map(|(time, _)| time)
            .collect();
        assert_eq!(times.len(), 4, "{:?}", script.log().events());
        let gaps: Vec<Duration> = times.windows(2).map(|pair| pair[1] - pair[0]).collect();
        for (gap, least) in gaps.iter().zip([20, 40, 80]) {
            assert!(*gap >= Duration::from_millis(least), "{gaps:?}");
        }
        release.stop().await;
    }

    // The clock is paused, so that a pause that is too long shows: a test that waited for real
    // could not tell 80 ms from 160 on a busy machine.
    #[tokio::test(start_paused = true)]
    async fn the_pauses_between_tries_stop_growing_at_the_most() {
        let holds = Holds::memory();
        left(&holds, 1, &[1]);
        let tries = Arc::new(std::sync::Mutex::new(Vec::new()));
        let script = Arc::new(Script::new(gives_up(&[1])).connects({
            let tries = Arc::clone(&tries);
            move |n| {
                lock(&tries).push(tokio::time::Instant::now());
                if n < 6 { Connect::Lost } else { Connect::Ok }
            }
        }));

        let release = Release::start(&holds, vec![(peer(1), script)], |_| true, NOW);
        release.until_nothing_is_held().await;

        // Five connections fail. The pauses after them are 20, 40 and 80 ms, and 80 again twice,
        // because that is the most.
        let tries = lock(&tries).clone();
        let gaps: Vec<Duration> = tries.windows(2).map(|pair| pair[1] - pair[0]).collect();
        assert_eq!(gaps.len(), 5, "{gaps:?}");
        for (gap, pause) in gaps.iter().zip([20, 40, 80, 80, 80]) {
            let pause = Duration::from_millis(pause);
            let slack = Duration::from_millis(15);
            assert!(*gap >= pause && *gap < pause + slack, "{gaps:?}");
        }
        release.stop().await;
    }

    // The clock is paused, as in the test above.
    #[tokio::test(start_paused = true)]
    async fn the_pauses_start_again_when_the_release_had_nothing_to_do_in_between() {
        let holds = Holds::memory();
        left(&holds, 1, &[1]);
        // The first device cannot be reached, until it is not paired any more.
        let paired = Arc::new(AtomicBool::new(true));
        let unreachable = Arc::new(Script::new(gives_up(&[1])).connects(|_| Connect::Lost));
        let tries = Arc::new(std::sync::Mutex::new(Vec::new()));
        let second = Arc::new(Script::new(gives_up(&[9])).connects({
            let tries = Arc::clone(&tries);
            move |n| {
                lock(&tries).push(tokio::time::Instant::now());
                if n < 2 { Connect::Lost } else { Connect::Ok }
            }
        }));
        let release = Release::start(
            &holds,
            vec![(peer(1), Arc::clone(&unreachable)), (peer(2), second)],
            {
                let paired = Arc::clone(&paired);
                move |device| *device != peer(1) || paired.load(Ordering::SeqCst)
            },
            NOW,
        );

        // The pauses grow to the most with the first device, 80 ms.
        until("the first device to have been tried four times", || {
            unreachable
                .log()
                .events()
                .iter()
                .filter(|event| matches!(event, Event::ConnectFailed(_)))
                .count()
                >= 4
        })
        .await;
        // Then it is not paired any more, what it held is dropped, and there is nothing to do.
        paired.store(false, Ordering::SeqCst);
        until("what the first device held to be dropped", || {
            holds.all().is_empty()
        })
        .await;

        // A transfer is left at the other device, which cannot be reached the first time. The pause
        // before the next try is the first one again, and not the most that the pauses had grown to.
        left(&holds, 2, &[9]);
        release.until_nothing_is_held().await;

        let tries = lock(&tries).clone();
        assert_eq!(tries.len(), 2, "{tries:?}");
        let gap = tries[1] - tries[0];
        assert!(
            gap >= Duration::from_millis(20) && gap < Duration::from_millis(35),
            "{gap:?}"
        );
        release.stop().await;
    }

    // The clock is paused, so that a retry with no pause shows as one.
    #[tokio::test(start_paused = true)]
    async fn an_answer_that_is_not_about_the_transfer_is_asked_again_after_a_pause() {
        let holds = Holds::memory();
        left(&holds, 1, &[1]);
        // The receiver answers as if it were another transfer, twice, and then properly.
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let script = Arc::new(Script::new({
            let asked = Arc::clone(&asked);
            move |call| {
                lock(&asked).push(tokio::time::Instant::now());
                if call.conn <= 2 {
                    cancelled(id(9))
                } else {
                    cancelled(id(1))
                }
            }
        }));

        let release = Release::start(&holds, vec![(peer(1), script)], |_| true, NOW);
        release.until_nothing_is_held().await;

        // The pauses are the ones that the pace says, 20 ms and then 40 ms, as after any other
        // try that did not end it.
        let asked = lock(&asked).clone();
        let gaps: Vec<Duration> = asked.windows(2).map(|pair| pair[1] - pair[0]).collect();
        assert_eq!(gaps.len(), 2, "{gaps:?}");
        for (gap, least) in gaps.iter().zip([20, 40]) {
            assert!(*gap >= Duration::from_millis(least), "{gaps:?}");
        }
        release.stop().await;
    }

    #[tokio::test]
    async fn a_receiver_that_does_not_answer_in_time_is_asked_again_later() {
        let holds = Holds::memory();
        left(&holds, 1, &[1]);
        let script = Arc::new(Script::new(|call| {
            if call.conn == 1 {
                Step::Silent
            } else {
                cancelled(id(1))
            }
        }));

        let release = Release::start(&holds, vec![(peer(1), Arc::clone(&script))], |_| true, NOW);
        release.until_nothing_is_held().await;

        assert_eq!(cancels_of(&script), [id(1), id(1)]);
        assert_eq!(connections(&script), 2);
        release.stop().await;
    }

    #[tokio::test]
    async fn an_answer_that_is_not_about_the_transfer_is_not_a_receiver_letting_go() {
        let holds = Holds::memory();
        left(&holds, 1, &[1, 2]);
        // The first is answered as if it were another transfer, and the second properly.
        let script = Arc::new(Script::new(|call| match call.nth {
            0 => cancelled(id(9)),
            _ => cancelled(id(2)),
        }));

        let release = Release::start(&holds, vec![(peer(1), Arc::clone(&script))], |_| true, NOW);
        until("the second to be given up", || holds.all().len() == 1).await;

        // The first is still held, and is asked again.
        assert_eq!(holds.all()[0].1, id(1));
        until("the first to be asked again", || {
            cancels_of(&script)
                .iter()
                .filter(|&&asked| asked == id(1))
                .count()
                >= 2
        })
        .await;
        assert_eq!(holds.all()[0].1, id(1));
        release.stop().await;
    }

    #[tokio::test]
    async fn a_connection_that_gets_no_answer_is_not_used_for_the_rest() {
        let holds = Holds::memory();
        left(&holds, 1, &[1, 2]);
        // The first connection is silent, and the second answers each request properly.
        let script = Arc::new(Script::new(|call| {
            if call.conn == 1 {
                Step::Silent
            } else {
                cancelled(id(u8::try_from(call.nth).unwrap() + 1))
            }
        }));

        let release = Release::start(&holds, vec![(peer(1), Arc::clone(&script))], |_| true, NOW);
        release.until_nothing_is_held().await;

        // The second transfer is not asked about on the connection that did not answer the first.
        assert_eq!(cancels_of(&script), [id(1), id(1), id(2)]);
        assert_eq!(connections(&script), 2);
        release.stop().await;
    }

    #[tokio::test]
    async fn a_receiver_that_cannot_make_sense_of_it_is_asked_again() {
        let holds = Holds::memory();
        left(&holds, 1, &[1]);
        let script = Arc::new(Script::new(|_| error(ErrorCode::Internal)));

        let release = Release::start(&holds, vec![(peer(1), Arc::clone(&script))], |_| true, NOW);
        until("two tries", || cancels_of(&script).len() >= 2).await;

        assert_eq!(holds.all().len(), 1);
        release.stop().await;
    }

    #[tokio::test]
    async fn a_device_that_turns_this_one_away_cannot_be_told_and_is_forgotten() {
        for refusal in [Connect::Refused(403), Connect::Ok] {
            let holds = Holds::memory();
            left(&holds, 1, &[1, 2]);
            left(&holds, 2, &[3]);
            let first = Arc::new(Script::new(|_| Step::Refused(403)).connects(move |_| refusal));
            let second = Arc::new(Script::new(gives_up(&[3])));

            let release = Release::start(
                &holds,
                vec![(peer(1), Arc::clone(&first)), (peer(2), second)],
                |_| true,
                NOW,
            );
            release.until_nothing_is_held().await;

            // Nothing more is tried: the other device is told nothing, however it turned this one away.
            let tries = cancels_of(&first).len();
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert_eq!(cancels_of(&first).len(), tries);
            assert!(tries <= 1, "{tries}");
            release.stop().await;
        }
    }

    #[tokio::test]
    async fn a_transfer_that_is_left_while_the_release_waits_is_given_up_at_once() {
        let holds = Holds::memory();
        let script = Arc::new(Script::new(gives_up(&[7])));
        let release = Release::start(&holds, vec![(peer(1), Arc::clone(&script))], |_| true, NOW);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(connections(&script), 0);

        holds.begin(peer(1), id(7), NOW);
        holds.ended(&peer(1), id(7), false, NOW);
        release.until_nothing_is_held().await;

        assert_eq!(cancels_of(&script), [id(7)]);
        release.stop().await;
    }

    #[tokio::test]
    async fn a_device_that_is_there_is_told_at_once_though_another_is_being_waited_for() {
        let holds = Holds::memory();
        left(&holds, 1, &[1]);
        let absent = Arc::new(Script::new(gives_up(&[1])).connects(|_| Connect::Lost));
        let present = Arc::new(Script::new(gives_up(&[2])));
        // The pause before the absent device is tried again is far longer than a test waits.
        let slow = Pace {
            start: Duration::from_secs(60),
            max: Duration::from_secs(60),
            late: Duration::from_secs(60),
            wait: Duration::from_millis(200),
        };
        let release = Release::paced(
            &holds,
            vec![
                (peer(1), Arc::clone(&absent)),
                (peer(2), Arc::clone(&present)),
            ],
            |_| true,
            NOW,
            slow,
        );
        until("the first try", || !absent.log().events().is_empty()).await;

        left(&holds, 2, &[2]);
        until("the second device to be told", || holds.all().len() == 1).await;

        assert_eq!(cancels_of(&present), [id(2)]);
        assert_eq!(holds.all(), [(peer(1), id(1), false)]);
        release.stop().await;
    }

    #[tokio::test]
    async fn a_device_that_does_not_answer_the_dial_does_not_hold_up_the_others() {
        let holds = Holds::memory();
        left(&holds, 1, &[1]);
        left(&holds, 2, &[2]);
        let hanging = Arc::new(Script::new(gives_up(&[1])).connects(|_| Connect::Hang));
        let present = Arc::new(Script::new(gives_up(&[2])));

        let release = Release::start(
            &holds,
            vec![(peer(1), hanging), (peer(2), Arc::clone(&present))],
            |_| true,
            NOW,
        );
        until("the second device to be told", || holds.all().len() == 1).await;

        assert_eq!(cancels_of(&present), [id(2)]);
        assert_eq!(holds.all(), [(peer(1), id(1), false)]);
        release.stop().await;
    }

    #[tokio::test]
    async fn a_transfer_that_is_going_is_never_given_up() {
        let holds = Holds::memory();
        holds.begin(peer(1), id(1), NOW);
        let script = Arc::new(Script::new(gives_up(&[1])));

        let release = Release::start(&holds, vec![(peer(1), Arc::clone(&script))], |_| true, NOW);
        tokio::time::sleep(Duration::from_millis(150)).await;

        assert_eq!(connections(&script), 0);
        assert_eq!(holds.all(), [(peer(1), id(1), true)]);
        release.stop().await;
    }

    #[tokio::test]
    async fn a_device_that_is_not_paired_is_not_dialed_and_what_it_held_is_dropped() {
        let holds = Holds::memory();
        left(&holds, 1, &[1]);
        let script = Arc::new(Script::new(gives_up(&[1])));

        let release = Release::start(&holds, vec![(peer(1), Arc::clone(&script))], |_| false, NOW);
        release.until_nothing_is_held().await;

        assert_eq!(connections(&script), 0);
        release.stop().await;
    }

    #[tokio::test]
    async fn a_transfer_that_was_left_long_ago_is_given_up_the_first_time_the_receiver_is_reached()
    {
        let holds = Holds::memory();
        left(&holds, 1, &[1]);
        let script = Arc::new(Script::new(gives_up(&[1])));

        // Far longer ago than a receiver keeps a transfer: it counts that hour on a clock that
        // stops while it sleeps, and may have slept through all of this time.
        let release = Release::start(
            &holds,
            vec![(peer(1), Arc::clone(&script))],
            |_| true,
            NOW + 100 * HELD_FOR.as_secs(),
        );
        release.until_nothing_is_held().await;

        assert_eq!(cancels_of(&script), [id(1)]);
        assert_eq!(connections(&script), 1);
        release.stop().await;
    }

    // The clock is paused, as in the tests of the pauses above.
    #[tokio::test(start_paused = true)]
    async fn the_pauses_grow_longer_once_what_was_left_is_more_than_an_hour_old() {
        let holds = Holds::memory();
        left(&holds, 1, &[1]);
        let tries = Arc::new(std::sync::Mutex::new(Vec::new()));
        let script = Arc::new(Script::new(gives_up(&[1])).connects({
            let tries = Arc::clone(&tries);
            move |n| {
                lock(&tries).push(tokio::time::Instant::now());
                if n < 7 { Connect::Lost } else { Connect::Ok }
            }
        }));

        // A receiver that stayed awake for the hour since the last word has let go. One that did
        // not, and is not there, is looked for less and less often, and never given up on.
        let now = NOW + HELD_FOR.as_secs() + 1;
        let release = Release::start(&holds, vec![(peer(1), script)], |_| true, now);
        release.until_nothing_is_held().await;

        // Six connections fail. The pauses after them are 20, 40, 80 and 160 ms, and 320 ms twice,
        // which is the most when nothing was left lately.
        let tries = lock(&tries).clone();
        let gaps: Vec<Duration> = tries.windows(2).map(|pair| pair[1] - pair[0]).collect();
        assert_eq!(gaps.len(), 6, "{gaps:?}");
        for (gap, pause) in gaps.iter().zip([20, 40, 80, 160, 320, 320]) {
            let pause = Duration::from_millis(pause);
            let slack = Duration::from_millis(15);
            assert!(*gap >= pause && *gap < pause + slack, "{gaps:?}");
        }
        release.stop().await;
    }

    // The clock is paused, as in the tests of the pauses above.
    #[tokio::test(start_paused = true)]
    async fn the_pauses_stay_short_while_one_of_the_transfers_was_left_lately() {
        let holds = Holds::memory();
        left(&holds, 1, &[1]);
        let now = NOW + HELD_FOR.as_secs() + 1;
        holds.begin(peer(1), id(2), now);
        holds.ended(&peer(1), id(2), false, now);
        let tries = Arc::new(std::sync::Mutex::new(Vec::new()));
        let script = Arc::new(Script::new(gives_up(&[1, 2])).connects({
            let tries = Arc::clone(&tries);
            move |n| {
                lock(&tries).push(tokio::time::Instant::now());
                if n < 6 { Connect::Lost } else { Connect::Ok }
            }
        }));

        let release = Release::start(&holds, vec![(peer(1), script)], |_| true, now);
        release.until_nothing_is_held().await;

        // The one that is old would let the pauses grow to 320 ms, and the other keeps them at 80.
        let tries = lock(&tries).clone();
        let gaps: Vec<Duration> = tries.windows(2).map(|pair| pair[1] - pair[0]).collect();
        assert_eq!(gaps.len(), 5, "{gaps:?}");
        for (gap, pause) in gaps.iter().zip([20, 40, 80, 80, 80]) {
            let pause = Duration::from_millis(pause);
            let slack = Duration::from_millis(15);
            assert!(*gap >= pause && *gap < pause + slack, "{gaps:?}");
        }
        release.stop().await;
    }

    #[tokio::test]
    async fn stopping_the_node_ends_the_release_whatever_it_is_waiting_for() {
        for connect in [Connect::Hang, Connect::Lost] {
            let holds = Holds::memory();
            left(&holds, 1, &[1]);
            let script = Arc::new(Script::new(gives_up(&[1])).connects(move |_| connect));
            let release =
                Release::start(&holds, vec![(peer(1), Arc::clone(&script))], |_| true, NOW);
            tokio::time::sleep(Duration::from_millis(60)).await;

            release.stop().await;

            assert_eq!(holds.all().len(), 1);
        }
    }

    #[tokio::test]
    async fn a_request_that_is_waited_for_when_the_node_stops_does_not_hold_it_back() {
        let holds = Holds::memory();
        left(&holds, 1, &[1]);
        let script = Arc::new(Script::new(|_| Step::Never));
        let release = Release::start(&holds, vec![(peer(1), Arc::clone(&script))], |_| true, NOW);
        until("the request", || !script.requests().is_empty()).await;

        release.stop().await;

        assert_eq!(holds.all().len(), 1);
    }
}
