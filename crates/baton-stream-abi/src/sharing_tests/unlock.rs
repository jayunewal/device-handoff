// SPDX-License-Identifier: Apache-2.0

//! Unlocking a locked Mac from the device that shows its screen (ADR 0013), whole, through the
//! functions of both platforms: the Mac behind [`FakeSharer`], which checks the password, puts it
//! in a box and types it, and the device that shows the screen behind [`Fake`], which makes the
//! key and asks for a face. The cryptography of the platforms is a stand-in: what is under test is
//! what the library does with the pieces, in which order, and what each platform is told.

use std::sync::Condvar;

use baton_app::{ErrorKind, ScreenStarted, UnlockResult, UnlockSetUp, UnlockStatus, WindowUnlock};

use super::*;
use crate::{
    abi::{
        BatonBytes, BatonUnlockBox, FACE_DECLINED, FACE_FAILED, FACE_KEY_LOST, KEEP_INVALID,
        KEEP_KEPT, KEEP_TWICE, SEALING_ECIES_P256, UNLOCK_BOX_MAX, UNLOCK_FAILED,
        UNLOCK_NOT_LOCKED, UNLOCK_NOT_PRIVATE, UNLOCK_REJECTED, UNLOCK_SEALED_MAX, UNLOCK_STALE,
        UNLOCK_UNLOCKED,
    },
    fake::{a_public_key, unlock_declined, unlock_opened, unlock_opened_raw},
    fake_sharer::{source_private, unlock_keep, unlock_keep_raw, unlock_live, unlock_live_to},
};

/// The password of the person at the Mac.
const PASSWORD: &str = "hunter2";

/// The box and the sealed key that the platform of the Mac keeps when it unlocks.
const NEW: (&[u8], &[u8]) = (b"box 2", b"sealed 2");

/// The id of a device, as the platforms are given it.
fn id_of(dev: &Dev) -> Vec<u8> {
    let id: DeviceId = dev.id().parse().unwrap();
    id.as_bytes().to_vec()
}

/// A private screen of the Mac that the other device shows.
struct Private {
    /// The session at the device that shows it.
    session: WindowSession,
    /// The number of the window at the sharing platform, which its calls are listed under.
    window: u64,
    /// The link that `start` gave the sharing platform.
    link: u64,
    /// The number of the screen at the showing platform, which its calls are listed under.
    screen: u64,
    /// The link that `open` gave the showing platform.
    screen_link: u64,
}

/// The question that the showing platform was given to answer with a face.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Asked {
    number: u64,
    sealed: Vec<u8>,
    id: Vec<u8>,
    name: String,
}

impl Rig {
    /// A Mac whose platform can let another device control and unlock its screen, a device that
    /// shows screens and unlocks them with a face, and a person at the Mac who lets that device
    /// look at the screen and control it.
    async fn unlockable() -> Self {
        Self::unlockable_with(|_, _| {}).await
    }

    /// As `unlockable`, with the platforms as `setup` leaves them.
    async fn unlockable_with(setup: impl FnOnce(&Arc<FakeSharer>, &Arc<Fake>)) -> Self {
        let rig = Self::with(|platform, viewer| {
            platform.set_control(true);
            setup(platform, viewer);
        })
        .await;
        rig.sharing
            .app
            .set_start_access(&rig.viewing.id(), StartAccess::Control)
            .unwrap();
        rig
    }

    /// The person at the Mac sets the unlock up for the other device, and it works.
    async fn sets_up(&self) {
        let done = self
            .sharing
            .app
            .set_up_unlock(&self.viewing.id(), PASSWORD.to_owned())
            .await
            .unwrap();
        assert_eq!(done, UnlockSetUp::Done);
    }

    /// The other device asks for the screen of the Mac, to control it, and shows its first
    /// picture. The platform of the Mac has not said that the privacy is in place.
    async fn screen_shown(&mut self) -> Private {
        let started = self
            .viewing
            .app
            .start_screen(&self.sharing.id(), true)
            .await
            .unwrap();
        let ScreenStarted::Started(session) = started else {
            panic!("the screen was not started: {started:?}");
        };
        let window = self.platform.windows_made() as u64;
        until("the platform to be started", || {
            self.platform
                .calls(window)
                .iter()
                .any(|call| matches!(call, SharerCall::Start { .. }))
        })
        .await;
        let link = self.platform.link(window);
        assert_eq!(
            self.send_laid_out(link, 0, window_layout()).await,
            (BEGIN_KEY, SENT, 0)
        );
        self.until_shown(0).await;
        let screen = self.viewer.screens_made() as u64;
        Private {
            session,
            window,
            link,
            screen,
            screen_link: self.screen_link(screen),
        }
    }

    /// The platform of the Mac says that the cover of the screens, the hold of the keys and the
    /// pointer, and the guard that locks the Mac if the app dies are all in place.
    fn private_in_place(&self, screen: &Private) {
        source_private(&self.registry, screen.link, 1).unwrap();
    }

    /// The Mac locks, and the other device is told.
    async fn locks(&mut self, screen: &Private) {
        pause(&self.registry, screen.link, PAUSE_LOCKED).unwrap();
        self.until_locked(screen, true).await;
    }

    /// The system says that the screen is unlocked, and the pictures go on.
    async fn unlocks(&mut self, screen: &Private) {
        resume(&self.registry, screen.link).unwrap();
        self.until_locked(screen, false).await;
    }

    async fn until_locked(&mut self, screen: &Private, locked: bool) {
        let id = screen.session.id;
        next_matching(&mut self.viewing_events, |event| match event {
            Event::WindowLocked {
                id: of,
                locked: now,
            } if of == id && now == locked => Some(()),
            _ => None,
        })
        .await;
    }

    /// The person at the other device taps Unlock. Whether the tap was taken.
    fn taps(&self, screen: &Private) -> bool {
        self.viewing.app.unlock_window(screen.session.id)
    }

    /// The tap is taken: a platform that declined a question told the person at once, and the
    /// library takes the answer a moment later.
    async fn taps_when_it_can(&self, screen: &Private) {
        until("the tap to be taken", || self.taps(screen)).await;
    }

    /// Waits until the showing platform has been given the `nth` question of the screen, counted
    /// from 1, and gives it.
    async fn question(&self, screen: &Private, nth: usize) -> Asked {
        let asked = || -> Vec<Asked> {
            self.screen_calls(screen.screen)
                .into_iter()
                .filter_map(|call| match call {
                    Call::UnlockOpen {
                        number,
                        sealed,
                        id,
                        name,
                    } => Some(Asked {
                        number,
                        sealed,
                        id,
                        name,
                    }),
                    _ => None,
                })
                .collect()
        };
        until("the platform to be asked for a face", || {
            asked().len() >= nth
        })
        .await;
        asked().remove(nth - 1)
    }

    /// How many times the showing platform was given a question.
    fn questions(&self, screen: &Private) -> usize {
        self.screen_calls(screen.screen)
            .iter()
            .filter(|call| matches!(call, Call::UnlockOpen { .. }))
            .count()
    }

    /// How many times the showing platform was told that the question it holds is not open.
    fn cancels(&self, screen: &Private) -> usize {
        self.screen_calls(screen.screen)
            .iter()
            .filter(|call| matches!(call, Call::UnlockCancel))
            .count()
    }

    /// The face of the person was recognised and opened the key of the box.
    fn opened(&self, screen: &Private, asked: &Asked, key: &[u8]) -> Result<(), Error> {
        unlock_opened(
            &self.viewing_registry,
            screen.screen_link,
            asked.number,
            key,
        )
    }

    /// The question was not opened, for this `FACE_` reason.
    fn declined(&self, screen: &Private, asked: &Asked, reason: u32) -> Result<(), Error> {
        unlock_declined(
            &self.viewing_registry,
            screen.screen_link,
            asked.number,
            reason,
        )
    }

    /// What the person at the other device is told of an unlock, in order, up to its end.
    async fn told(&mut self, screen: &Private) -> Vec<WindowUnlock> {
        let id = screen.session.id;
        let mut told = Vec::new();
        loop {
            let step = next_matching(&mut self.viewing_events, |event| match event {
                Event::WindowUnlock { id: of, step } if of == id => Some(step),
                _ => None,
            })
            .await;
            told.push(step);
            if matches!(step, WindowUnlock::Ended(_)) {
                return told;
            }
        }
    }

    /// One whole attempt, to the point where the face of the person opens the key: the person taps,
    /// the platform is asked for a face, and the face answers with `key`. Gives the question and
    /// what the person is told.
    async fn attempt(
        &mut self,
        screen: &Private,
        nth: usize,
        key: &[u8; 32],
    ) -> (Asked, Vec<WindowUnlock>) {
        self.taps_when_it_can(screen).await;
        let asked = self.question(screen, nth).await;
        self.opened(screen, &asked, key).unwrap();
        let told = self.told(screen).await;
        (asked, told)
    }

    /// The next attempt to unlock the Mac that ended, as the person at the Mac is told.
    async fn attempt_told(&mut self) -> baton_app::UnlockEntry {
        next_matching(&mut self.sharing_events, |event| match event {
            Event::UnlockAttempted(entry) => Some(entry),
            _ => None,
        })
        .await
    }

    /// The person at the other device ends the screen, and everything is let go of.
    async fn ends(&self, screen: &Private) {
        assert!(self.viewing.app.end_window(screen.session.id));
        self.nothing_is_left().await;
    }
}

/// What the platform of the Mac was told to unlock the screen of the window, in order.
fn unlocks_of(rig: &Rig, screen: &Private) -> Vec<SharerCall> {
    rig.platform
        .calls(screen.window)
        .into_iter()
        .filter(|call| matches!(call, SharerCall::Unlock { .. }))
        .collect()
}

/// What the platform of the Mac is told to unlock the screen with, when the face opened `key` and
/// the box that is kept is `boxed`.
fn unlock_call(key: u8, boxed: &[u8], mac: &[u8], pad: &[u8]) -> SharerCall {
    SharerCall::Unlock {
        sealing: SEALING_ECIES_P256,
        box_key: vec![key; 32],
        boxed: boxed.to_vec(),
        viewer_key: a_public_key(),
        target: mac.to_vec(),
        viewer: pad.to_vec(),
    }
}

/// A platform that finds the attempt live, keeps a new box if it is given one, and reports `code`.
fn unlocking(
    registry: &Arc<Registry>,
    new: Option<(&'static [u8], &'static [u8])>,
    code: u32,
) -> impl Fn(u64, &SharerCall) -> u32 + Send + Sync + 'static {
    let registry = Arc::clone(registry);
    move |link, _| {
        assert_eq!(unlock_live(&registry, link), Ok(true));
        if let Some((boxed, sealed)) = new {
            assert_eq!(unlock_keep(&registry, link, boxed, sealed), Ok(KEEP_KEPT));
        }
        code
    }
}

/// What a call back into the library from inside `unlock` was named, and what it answered.
type Answer = (&'static str, Result<(), Error>);

/// What a call to keep the new box was named, and what it answered: a `KEEP_` number.
type Said = (&'static str, Result<u32, Error>);

/// Calls to keep the new box with pointers that are not there and structs that are not whole.
fn wrong_keeps(registry: &Registry, link: u64) -> Vec<Said> {
    let keep = BatonUnlockBox {
        size: size_of_u32::<BatonUnlockBox>(),
        reserved: 0,
        boxed: BatonBytes::of(b"box 2"),
        sealed: BatonBytes::of(b"sealed 2"),
    };
    let short = BatonUnlockBox { size: 8, ..keep };
    let nowhere = BatonUnlockBox {
        boxed: BatonBytes {
            ptr: ptr::null(),
            len: 3,
        },
        ..keep
    };
    let mut result = u32::MAX;
    // SAFETY: each is null, or a struct of the interface that lives through the call, with bytes
    // that do too, and the one with bytes that are not there is refused unread.
    let wrong = unsafe {
        [
            (
                "no struct",
                unlock_keep_raw(registry, link, None, Some(&mut result)),
            ),
            (
                "no place for the answer",
                unlock_keep_raw(registry, link, Some(&keep), None),
            ),
            (
                "a struct that ends too soon",
                unlock_keep_raw(registry, link, Some(&short), Some(&mut result)),
            ),
            (
                "bytes that are not there",
                unlock_keep_raw(registry, link, Some(&nowhere), Some(&mut result)),
            ),
        ]
    };
    let mut heard: Vec<Said> = wrong
        .into_iter()
        .map(|(name, answer)| (name, answer.map(|()| result)))
        .collect();
    heard.push((
        "no place for the answer to live",
        unlock_live_to(registry, link, false).map(|()| result),
    ));
    heard
}

/// Calls to keep a new box or a sealed key that is empty or beyond its room.
fn invalid_keeps(registry: &Registry, link: u64) -> Vec<Said> {
    let (box_max, sealed_max) = (UNLOCK_BOX_MAX as usize, UNLOCK_SEALED_MAX as usize);
    let invalid: [(&'static str, Vec<u8>, Vec<u8>); 5] = [
        ("an empty box", vec![], b"sealed".to_vec()),
        ("an empty sealed key", b"box".to_vec(), vec![]),
        (
            "a box one byte too large",
            vec![1; box_max + 1],
            b"sealed".to_vec(),
        ),
        (
            "a sealed key one byte too large",
            b"box".to_vec(),
            vec![1; sealed_max + 1],
        ),
        ("a box far too large", vec![1; 5000], b"sealed".to_vec()),
    ];
    invalid
        .into_iter()
        .map(|(name, boxed, sealed)| (name, unlock_keep(registry, link, &boxed, &sealed)))
        .collect()
}

/// While no unlock runs, the two functions of an unlock find no attempt on any link.
fn nothing_to_ask_or_keep(rig: &Rig, link: u64) {
    assert_eq!(unlock_live(&rig.registry, link), Err(Error::UnknownLink));
    assert_eq!(
        unlock_keep(&rig.registry, link, b"box", b"sealed"),
        Err(Error::UnknownLink)
    );
}

/// What the platform inside `unlock` and the test tell each other.
#[derive(Default)]
struct Waiting {
    entered: bool,
    open: bool,
}

/// The platform waits inside `unlock` until the test lets it go.
#[derive(Clone, Default)]
struct Hold(Arc<(Mutex<Waiting>, Condvar)>);

impl Hold {
    /// Called by the platform: says that it is inside the call, and waits until it is let go.
    fn wait(&self) {
        let (state, changed) = &*self.0;
        let mut state = state.lock().unwrap();
        state.entered = true;
        changed.notify_all();
        // A test that fails does not leave the thread waiting for ever.
        let _ = changed
            .wait_timeout_while(state, WAIT, |state| !state.open)
            .unwrap();
    }

    /// Whether the platform is inside the call.
    fn entered(&self) -> bool {
        self.0.0.lock().unwrap().entered
    }

    /// Lets the platform go.
    fn let_go(&self) {
        let (state, changed) = &*self.0;
        state.lock().unwrap().open = true;
        changed.notify_all();
    }
}

/// Lets the platform go when the test ends, also by a failure.
struct LetsGoAtTheEnd(Hold);

impl Drop for LetsGoAtTheEnd {
    fn drop(&mut self) {
        self.0.let_go();
    }
}

#[tokio::test]
async fn a_locked_mac_is_unlocked_through_both_platforms_and_each_unlock_opens_the_box_the_one_before_kept()
 {
    let mut rig = Rig::unlockable().await;
    let (mac, pad) = (id_of(&rig.sharing), id_of(&rig.viewing));

    // The set-up: the platform of the Mac checks the password, the platform of the other device
    // makes a new key for the Mac, and the platform of the Mac puts the password in a box that only
    // that key opens, bound to the two devices.
    rig.sets_up().await;
    let heard: Vec<_> = rig
        .platform
        .log()
        .into_iter()
        .map(|(_, call)| call)
        .filter(|call| {
            matches!(
                call,
                SharerCall::UnlockCheck { .. } | SharerCall::UnlockSetup { .. }
            )
        })
        .collect();
    assert_eq!(
        heard,
        [
            SharerCall::UnlockCheck {
                password: PASSWORD.into()
            },
            SharerCall::UnlockSetup {
                password: PASSWORD.into(),
                sealing: SEALING_ECIES_P256,
                target: mac.clone(),
                viewer: pad.clone(),
                viewer_key: a_public_key(),
            },
        ]
    );
    let keys: Vec<_> = rig
        .viewer
        .log()
        .into_iter()
        .filter_map(|(_, call)| match call {
            Call::UnlockKey { id, name, renew } => Some((id, name, renew)),
            _ => None,
        })
        .collect();
    assert_eq!(keys, [(mac.clone(), "Studio Mac".to_owned(), true)]);

    // The other device shows the screen, which takes an unlock, and the Mac locks.
    let screen = rig.screen_shown().await;
    assert!(screen.session.unlock, "{:?}", screen.session);
    rig.private_in_place(&screen);
    rig.platform
        .plan_unlock(unlocking(&rig.registry, Some(NEW), UNLOCK_UNLOCKED));
    rig.locks(&screen).await;

    // The person taps Unlock, and the platform is asked for a face, in words that name the Mac,
    // with the key of the box that the Mac sealed to the key that the platform made.
    assert!(rig.taps(&screen));
    let asked = rig.question(&screen, 1).await;
    let question = Asked {
        number: 1,
        sealed: b"a sealed key".to_vec(),
        id: mac.clone(),
        name: "Studio Mac".to_owned(),
    };
    assert_eq!(asked, question);
    // A second tap while the first goes is not taken.
    assert!(!rig.taps(&screen));

    // The face opens the key, and the platform of the Mac is given it, with the box and the keys
    // and the ids that the box is bound to.
    rig.opened(&screen, &asked, &[9; 32]).unwrap();
    assert_eq!(
        rig.told(&screen).await,
        [
            WindowUnlock::Asking,
            WindowUnlock::Typing,
            WindowUnlock::Ended(UnlockResult::Unlocked)
        ]
    );
    assert_eq!(
        unlocks_of(&rig, &screen),
        [unlock_call(9, b"a box", &mac, &pad)]
    );
    let entry = rig.attempt_told().await;
    assert_eq!(entry.result, UnlockResult::Unlocked);
    assert_eq!(entry.device_id, rig.viewing.id());

    // The screen is unlocked. Locked again, the next question holds the key that the platform
    // kept, and the platform is given the box that it kept: what opened the first opens nothing.
    rig.unlocks(&screen).await;
    rig.platform.plan_unlock(unlocking(
        &rig.registry,
        Some((b"box 3", b"sealed 3")),
        UNLOCK_UNLOCKED,
    ));
    rig.locks(&screen).await;
    let (asked, told) = rig.attempt(&screen, 2, &[8; 32]).await;
    assert_eq!((asked.number, asked.sealed), (2, b"sealed 2".to_vec()));
    assert_eq!(
        told.last(),
        Some(&WindowUnlock::Ended(UnlockResult::Unlocked))
    );
    assert_eq!(
        unlocks_of(&rig, &screen)[1],
        unlock_call(8, b"box 2", &mac, &pad)
    );

    rig.ends(&screen).await;
}

#[tokio::test]
async fn each_outcome_that_the_platform_reports_is_what_the_person_at_the_other_device_is_told() {
    // What the platform returns, whether it kept a new box before it did, and what the person is
    // told. The ones that follow a new box have one.
    for (code, keeps, told) in [
        (UNLOCK_UNLOCKED, true, UnlockResult::Unlocked),
        (UNLOCK_REJECTED, true, UnlockResult::Rejected),
        (UNLOCK_STALE, false, UnlockResult::Stale),
        (UNLOCK_FAILED, false, UnlockResult::Failed),
        (UNLOCK_NOT_LOCKED, true, UnlockResult::NotLocked),
        (UNLOCK_NOT_PRIVATE, true, UnlockResult::NotPrivate),
        // A number that this version of the interface does not have is a failure.
        (99, false, UnlockResult::Failed),
        (u32::MAX, false, UnlockResult::Failed),
    ] {
        let mut rig = Rig::unlockable().await;
        rig.sets_up().await;
        let screen = rig.screen_shown().await;
        rig.private_in_place(&screen);
        rig.platform
            .plan_unlock(unlocking(&rig.registry, keeps.then_some(NEW), code));
        rig.locks(&screen).await;

        let (_, steps) = rig.attempt(&screen, 1, &[9; 32]).await;
        assert_eq!(
            steps,
            [
                WindowUnlock::Asking,
                WindowUnlock::Typing,
                WindowUnlock::Ended(told)
            ],
            "the platform returned {code}"
        );
        // The person at the Mac is told the same, but for the platform that reports a number that
        // it has no right to.
        assert_eq!(rig.attempt_told().await.result, told, "{code}");
        rig.ends(&screen).await;
    }
}

#[tokio::test]
async fn a_platform_that_reports_the_screen_unlocked_without_a_new_box_ends_the_setup() {
    let mut rig = Rig::unlockable().await;
    rig.sets_up().await;
    let screen = rig.screen_shown().await;
    rig.private_in_place(&screen);
    // The platform typed, and kept nothing: the key that was sent would open the box that the Mac
    // holds, and a copy of it would be good for one more unlock.
    rig.platform
        .plan_unlock(unlocking(&rig.registry, None, UNLOCK_UNLOCKED));
    rig.locks(&screen).await;
    let (_, steps) = rig.attempt(&screen, 1, &[9; 32]).await;
    // The screen is unlocked, and the person is told so.
    assert_eq!(
        steps.last(),
        Some(&WindowUnlock::Ended(UnlockResult::Unlocked))
    );
    // The Mac's history says what the platform did wrong, and the unlock has ended.
    assert_eq!(rig.attempt_told().await.result, UnlockResult::Failed);
    assert!(matches!(
        rig.sharing.app.unlock_status(&rig.viewing.id()).unwrap(),
        UnlockStatus::Ended { .. }
    ));

    // The next try is refused before anything is asked of the person.
    rig.unlocks(&screen).await;
    rig.locks(&screen).await;
    rig.taps_when_it_can(&screen).await;
    assert_eq!(
        rig.told(&screen).await,
        [WindowUnlock::Ended(UnlockResult::NotSetUp)]
    );
    assert_eq!(rig.questions(&screen), 1);
    assert_eq!(unlocks_of(&rig, &screen).len(), 1);
    rig.ends(&screen).await;
}

#[tokio::test]
async fn a_mac_is_not_unlocked_unless_its_platform_last_said_that_the_privacy_is_in_place() {
    let mut rig = Rig::unlockable().await;
    rig.sets_up().await;
    let screen = rig.screen_shown().await;
    rig.platform
        .plan_unlock(unlocking(&rig.registry, Some(NEW), UNLOCK_UNLOCKED));
    rig.locks(&screen).await;

    // The platform has said nothing, or something other than a one: the Mac does not ask for a
    // face, the person is told so, and the platform is never called.
    for word in [None, Some(0), Some(2), Some(u32::MAX)] {
        if let Some(word) = word {
            source_private(&rig.registry, screen.link, word).unwrap();
        }
        rig.taps_when_it_can(&screen).await;
        assert_eq!(
            rig.told(&screen).await,
            [WindowUnlock::Ended(UnlockResult::NotPrivate)],
            "the word was {word:?}"
        );
    }
    assert_eq!(rig.questions(&screen), 0);
    assert_eq!(unlocks_of(&rig, &screen), []);

    // Said in so many words, it goes on.
    rig.private_in_place(&screen);
    let (_, steps) = rig.attempt(&screen, 1, &[9; 32]).await;
    assert_eq!(
        steps.last(),
        Some(&WindowUnlock::Ended(UnlockResult::Unlocked))
    );

    // And the latest word counts: the guard that locks the Mac if the app dies is gone.
    rig.unlocks(&screen).await;
    rig.locks(&screen).await;
    source_private(&rig.registry, screen.link, 0).unwrap();
    rig.taps_when_it_can(&screen).await;
    assert_eq!(
        rig.told(&screen).await,
        [WindowUnlock::Ended(UnlockResult::NotPrivate)]
    );
    assert_eq!(rig.questions(&screen), 1);
    assert_eq!(unlocks_of(&rig, &screen).len(), 1);
    rig.ends(&screen).await;
}

#[tokio::test]
async fn what_the_other_device_lifts_of_the_privacy_ends_the_unlock_before_a_face_is_asked_for() {
    let mut rig = Rig::unlockable().await;
    rig.sets_up().await;
    let screen = rig.screen_shown().await;
    rig.private_in_place(&screen);
    rig.platform
        .plan_unlock(unlocking(&rig.registry, Some(NEW), UNLOCK_UNLOCKED));
    rig.locks(&screen).await;
    let id = screen.session.id;

    // Whatever the platform says, the person at the other device lifted the cover, or the hold of
    // the keys and the pointer, and the Mac is not private any more.
    for (cover, hold) in [(true, false), (false, true), (true, true)] {
        assert!(rig.viewing.app.window_lift(id, cover, hold));
        rig.taps_when_it_can(&screen).await;
        assert_eq!(
            rig.told(&screen).await,
            [WindowUnlock::Ended(UnlockResult::NotPrivate)],
            "lifted: cover {cover}, hold {hold}"
        );
    }
    assert_eq!(rig.questions(&screen), 0);

    // Put back, the unlock goes on.
    assert!(rig.viewing.app.window_lift(id, false, false));
    let (_, steps) = rig.attempt(&screen, 1, &[9; 32]).await;
    assert_eq!(
        steps.last(),
        Some(&WindowUnlock::Ended(UnlockResult::Unlocked))
    );
    rig.ends(&screen).await;
}

#[tokio::test]
async fn inside_unlock_the_platform_may_call_three_functions_of_the_library_and_no_others() {
    let mut rig = Rig::unlockable().await;
    rig.sets_up().await;
    let screen = rig.screen_shown().await;
    rig.private_in_place(&screen);
    let answers: Arc<Mutex<Vec<Answer>>> = Arc::default();
    {
        let (registry, answers) = (Arc::clone(&rig.registry), Arc::clone(&answers));
        rig.platform.plan_unlock(move |link, _| {
            // A call that was let in could wait for a lock that the library holds for this one.
            let _watch = Watchdog::start("a call back into the library");
            let mut heard = every_call(&registry, link);
            // The three that are for this call work.
            heard.push(("source_private", source_private(&registry, link, 1)));
            heard.push(("unlock_live", unlock_live(&registry, link).map(drop)));
            heard.push((
                "unlock_keep",
                unlock_keep(&registry, link, b"box 2", b"sealed 2").map(drop),
            ));
            answers.lock().unwrap().extend(heard);
            UNLOCK_UNLOCKED
        });
    }
    rig.locks(&screen).await;
    let (_, steps) = rig.attempt(&screen, 1, &[9; 32]).await;
    assert_eq!(
        steps.last(),
        Some(&WindowUnlock::Ended(UnlockResult::Unlocked))
    );
    // The box that the platform kept from inside the call was kept: the attempt is not one that
    // typed and kept nothing.
    assert_eq!(rig.attempt_told().await.result, UnlockResult::Unlocked);

    let answers = std::mem::take(&mut *answers.lock().unwrap());
    let (refused, allowed) = answers.split_at(answers.len() - 3);
    assert!(refused.len() >= 14, "{} answers", refused.len());
    for (name, answer) in refused {
        assert_eq!(*answer, Err(Error::Reentrant), "{name} inside unlock");
    }
    for (name, answer) in allowed {
        assert_eq!(*answer, Ok(()), "{name} inside unlock");
    }
    rig.ends(&screen).await;
}

#[tokio::test]
async fn what_became_of_keeping_the_new_box_is_told_to_the_platform_by_number_and_a_wrong_call_is_refused()
 {
    let mut rig = Rig::unlockable().await;
    rig.sets_up().await;
    let screen = rig.screen_shown().await;
    rig.private_in_place(&screen);
    let said: Arc<Mutex<Vec<Said>>> = Arc::default();
    {
        let (registry, said) = (Arc::clone(&rig.registry), Arc::clone(&said));
        rig.platform.plan_unlock(move |link, _| {
            let mut heard = wrong_keeps(&registry, link);
            heard.extend(invalid_keeps(&registry, link));
            // The largest that is allowed is kept, once.
            let (box_max, sealed_max) = (UNLOCK_BOX_MAX as usize, UNLOCK_SEALED_MAX as usize);
            heard.push((
                "the largest box and sealed key",
                unlock_keep(&registry, link, &vec![2; box_max], &vec![2; sealed_max]),
            ));
            heard.push((
                "a second box",
                unlock_keep(&registry, link, b"box 3", b"sealed 3"),
            ));
            *said.lock().unwrap() = heard;
            UNLOCK_UNLOCKED
        });
    }
    rig.locks(&screen).await;
    let (_, steps) = rig.attempt(&screen, 1, &[9; 32]).await;
    assert_eq!(
        steps.last(),
        Some(&WindowUnlock::Ended(UnlockResult::Unlocked))
    );
    let wrong = |name| (name, Err(Error::Argument));
    let invalid = |name| (name, Ok(KEEP_INVALID));
    assert_eq!(
        *said.lock().unwrap(),
        [
            wrong("no struct"),
            wrong("no place for the answer"),
            wrong("a struct that ends too soon"),
            wrong("bytes that are not there"),
            wrong("no place for the answer to live"),
            invalid("an empty box"),
            invalid("an empty sealed key"),
            invalid("a box one byte too large"),
            invalid("a sealed key one byte too large"),
            invalid("a box far too large"),
            ("the largest box and sealed key", Ok(KEEP_KEPT)),
            ("a second box", Ok(KEEP_TWICE)),
        ]
    );
    rig.ends(&screen).await;
}

#[tokio::test]
async fn outside_an_unlock_the_platform_has_nothing_to_ask_or_keep() {
    let mut rig = Rig::unlockable().await;
    rig.sets_up().await;
    let screen = rig.screen_shown().await;
    rig.private_in_place(&screen);
    rig.platform
        .plan_unlock(unlocking(&rig.registry, Some(NEW), UNLOCK_UNLOCKED));
    // Before any unlock, whatever the link is.
    for link in [screen.link, 0, u64::MAX, screen.link + 1000] {
        nothing_to_ask_or_keep(&rig, link);
    }
    // And once one is over: the attempt is not left where the platform's calls find it.
    rig.locks(&screen).await;
    let (_, steps) = rig.attempt(&screen, 1, &[9; 32]).await;
    assert_eq!(
        steps.last(),
        Some(&WindowUnlock::Ended(UnlockResult::Unlocked))
    );
    nothing_to_ask_or_keep(&rig, screen.link);
    // The word about the privacy is for a window that is open.
    for link in [0, u64::MAX, screen.link + 1000] {
        assert_eq!(
            source_private(&rig.registry, link, 1),
            Err(Error::UnknownLink)
        );
    }
    assert_eq!(source_private(&rig.registry, screen.link, 1), Ok(()));
    rig.ends(&screen).await;
}

#[tokio::test]
async fn what_the_person_does_while_the_platform_unlocks_is_dropped_as_the_mac_being_locked() {
    let mut rig = Rig::unlockable().await;
    rig.sets_up().await;
    let screen = rig.screen_shown().await;
    rig.private_in_place(&screen);
    // The platform takes its time inside `unlock`.
    let hold = Hold::default();
    let _lets_go = LetsGoAtTheEnd(hold.clone());
    {
        let (registry, hold) = (Arc::clone(&rig.registry), hold.clone());
        rig.platform.plan_unlock(move |link, call| {
            hold.wait();
            unlocking(&registry, Some(NEW), UNLOCK_UNLOCKED)(link, call)
        });
    }
    rig.locks(&screen).await;
    assert!(rig.taps(&screen));
    let asked = rig.question(&screen, 1).await;
    rig.opened(&screen, &asked, &[9; 32]).unwrap();
    until("the platform to be inside unlock", || hold.entered()).await;
    // The attempt is where the platform's calls find it, from any thread.
    assert_eq!(unlock_live(&rig.registry, screen.link), Ok(true));

    // The thread that serves the window is busy, so what the person does meanwhile waits.
    let id = screen.session.id;
    let input = rig.registry.source(screen.link).unwrap().input();
    for event in [
        touch(0.5, 0.5, WindowPointer::Down(WindowButton::Left)),
        touch(0.5, 0.5, WindowPointer::Up(WindowButton::Left)),
        typed(0x04, true, Some("a")),
    ] {
        assert!(rig.viewing.app.window_input(id, event));
    }
    until("the events to wait for the unlock", || {
        input.queued_events() == 3
    })
    .await;
    assert_eq!(rig.platform.posts(screen.window), []);

    // They were meant for a locked screen, and the screen is not. They are given up when the
    // platform returns, and the person is told that the Mac was locked.
    hold.let_go();
    assert_eq!(rig.notice().await, (id, WindowNoticeReason::Locked, 3));
    assert_eq!(rig.attempt_told().await.result, UnlockResult::Unlocked);
    assert_eq!(rig.platform.posts(screen.window), []);
    assert_eq!(input.queued_events(), 0);

    // The thread serves the window again: what the person does next is put to it.
    rig.unlocks(&screen).await;
    assert!(
        rig.viewing
            .app
            .window_input(id, touch(0.5, 0.5, WindowPointer::Move))
    );
    until("the window to be done to", || {
        rig.platform.posts(screen.window).len() == 1
    })
    .await;
    rig.ends(&screen).await;
}

#[tokio::test]
async fn a_window_that_ends_while_the_platform_unlocks_closes_its_link_so_the_attempt_is_over() {
    type Heard = (Result<bool, Error>, Result<u32, Error>);
    let mut rig = Rig::unlockable().await;
    rig.sets_up().await;
    let screen = rig.screen_shown().await;
    rig.private_in_place(&screen);
    let hold = Hold::default();
    let _lets_go = LetsGoAtTheEnd(hold.clone());
    let seen: Arc<Mutex<Option<Heard>>> = Arc::default();
    {
        let (registry, hold, seen) = (Arc::clone(&rig.registry), hold.clone(), Arc::clone(&seen));
        rig.platform.plan_unlock(move |link, _| {
            hold.wait();
            // What the platform does before it keeps the new box and before each key.
            let live = unlock_live(&registry, link);
            let kept = unlock_keep(&registry, link, b"box 2", b"sealed 2");
            *seen.lock().unwrap() = Some((live, kept));
            UNLOCK_FAILED
        });
    }
    rig.locks(&screen).await;
    assert!(rig.taps(&screen));
    let asked = rig.question(&screen, 1).await;
    rig.opened(&screen, &asked, &[9; 32]).unwrap();
    until("the platform to be inside unlock", || hold.entered()).await;

    // The person at the other device closes the screen, and the Mac hears of it. The session
    // closes its link as it ends, also while the platform is still inside unlock.
    assert!(rig.viewing.app.end_window(screen.session.id));
    rig.sharing_end().await;
    until("the link to be closed", || rig.registry.open_links() == 0).await;
    hold.let_go();
    rig.nothing_is_left().await;
    // Both calls are about a session that is over, which is how the header says that the attempt
    // is: the platform types nothing and returns.
    assert_eq!(
        *seen.lock().unwrap(),
        Some((Err(Error::UnknownLink), Err(Error::UnknownLink)))
    );
    // The platform was told that the session is over once the call returned, and nothing was
    // posted to the window.
    let calls = rig.platform.calls(screen.window);
    assert_eq!(
        calls[calls.len() - 2..],
        [SharerCall::Ended, SharerCall::Release]
    );
    assert_eq!(rig.platform.posts(screen.window), []);
}

#[tokio::test]
async fn a_face_answers_the_question_with_its_number_once_and_any_other_answer_is_refused() {
    let mut rig = Rig::unlockable().await;
    rig.sets_up().await;
    let screen = rig.screen_shown().await;
    rig.private_in_place(&screen);
    rig.platform
        .plan_unlock(unlocking(&rig.registry, Some(NEW), UNLOCK_UNLOCKED));
    rig.locks(&screen).await;
    assert!(rig.taps(&screen));
    let asked = rig.question(&screen, 1).await;

    // An answer for a question that is not open, or on a link that is not the screen's, is
    // refused, and the question that is open stays as it was.
    let key = [9; 32];
    for number in [0, asked.number + 1, u64::MAX] {
        let screen_link = screen.screen_link;
        assert_eq!(
            unlock_opened(&rig.viewing_registry, screen_link, number, &key),
            Err(Error::UnknownLink),
            "opened, question {number}"
        );
        assert_eq!(
            unlock_declined(&rig.viewing_registry, screen_link, number, FACE_DECLINED),
            Err(Error::UnknownLink),
            "declined, question {number}"
        );
    }
    for link in [0, u64::MAX, screen.screen_link + 1000] {
        let number = asked.number;
        assert_eq!(
            unlock_opened(&rig.viewing_registry, link, number, &key),
            Err(Error::UnknownLink),
            "opened, link {link}"
        );
        assert_eq!(
            unlock_declined(&rig.viewing_registry, link, number, FACE_DECLINED),
            Err(Error::UnknownLink),
            "declined, link {link}"
        );
    }
    assert_eq!(rig.cancels(&screen), 0);

    // The answer is taken once. Another, of either kind, is late.
    rig.opened(&screen, &asked, &key).unwrap();
    assert_eq!(rig.opened(&screen, &asked, &key), Err(Error::UnknownLink));
    assert_eq!(
        rig.declined(&screen, &asked, FACE_DECLINED),
        Err(Error::UnknownLink)
    );
    assert_eq!(
        rig.told(&screen).await,
        [
            WindowUnlock::Asking,
            WindowUnlock::Typing,
            WindowUnlock::Ended(UnlockResult::Unlocked)
        ]
    );
    // The question is closed by its answer: the platform is not told to close it.
    assert_eq!(rig.cancels(&screen), 0);
    rig.ends(&screen).await;
}

#[tokio::test]
async fn a_key_that_is_not_a_key_of_a_box_declines_the_question_and_the_attempt_gives_up() {
    let mut rig = Rig::unlockable().await;
    rig.sets_up().await;
    let screen = rig.screen_shown().await;
    rig.private_in_place(&screen);
    rig.locks(&screen).await;

    let (short, long, huge) = ([1; 31], [1; 33], vec![1; 4096]);
    let nowhere = BatonBytes {
        ptr: ptr::null(),
        len: 32,
    };
    let wrong = [
        ("31 bytes", Some(BatonBytes::of(&short))),
        ("33 bytes", Some(BatonBytes::of(&long))),
        ("no bytes", Some(BatonBytes::of(&[]))),
        ("4096 bytes", Some(BatonBytes::of(&huge))),
        ("32 bytes that are not there", Some(nowhere)),
        ("no struct", None),
    ];
    for (n, (what, key)) in wrong.iter().enumerate() {
        // Each is a try of its own: the person taps, and the platform answers wrong.
        rig.taps_when_it_can(&screen).await;
        let asked = rig.question(&screen, n + 1).await;
        // SAFETY: null, or a struct that lives through the call, with bytes that do too, but for
        // the one that says that its bytes are somewhere, which the library refuses unread.
        let answer = unsafe {
            unlock_opened_raw(
                &rig.viewing_registry,
                screen.screen_link,
                asked.number,
                key.as_ref(),
            )
        };
        assert_eq!(answer, Err(Error::Argument), "{what}");
        // The person is waiting for an answer, and is told that the attempt failed.
        assert_eq!(
            rig.told(&screen).await,
            [
                WindowUnlock::Asking,
                WindowUnlock::Ended(UnlockResult::GaveUp)
            ],
            "{what}"
        );
        assert_eq!(
            rig.opened(&screen, &asked, &[9; 32]),
            Err(Error::UnknownLink)
        );
    }
    assert_eq!(unlocks_of(&rig, &screen), []);
    rig.ends(&screen).await;
}

#[tokio::test]
async fn each_reason_that_a_face_was_not_given_is_what_the_person_is_told_of_the_attempt() {
    // The reason the platform gives, and what the person is told. A reason that this version
    // does not know is a failure, as the library does not know what the platform means by it.
    for (reason, told) in [
        (FACE_DECLINED, UnlockResult::Declined),
        (FACE_FAILED, UnlockResult::GaveUp),
        (FACE_KEY_LOST, UnlockResult::KeyLost),
        (77, UnlockResult::GaveUp),
        (u32::MAX, UnlockResult::GaveUp),
    ] {
        let mut rig = Rig::unlockable().await;
        rig.sets_up().await;
        let screen = rig.screen_shown().await;
        rig.private_in_place(&screen);
        rig.locks(&screen).await;
        assert!(rig.taps(&screen));
        let asked = rig.question(&screen, 1).await;
        rig.declined(&screen, &asked, reason).unwrap();
        assert_eq!(
            rig.told(&screen).await,
            [WindowUnlock::Asking, WindowUnlock::Ended(told)],
            "the reason was {reason}"
        );
        assert_eq!(
            rig.declined(&screen, &asked, reason),
            Err(Error::UnknownLink)
        );
        assert_eq!(unlocks_of(&rig, &screen), []);

        // A key that the phone lost ends the set-up, and the Mac says so before it asks anything
        // else. Any other reason leaves the unlock as it was.
        rig.taps_when_it_can(&screen).await;
        if told == UnlockResult::KeyLost {
            assert_eq!(
                rig.told(&screen).await,
                [WindowUnlock::Ended(UnlockResult::NotSetUp)]
            );
            // The refusal comes from what the Mac holds in memory. The record is ended in the
            // same write as the line of the history, which lands a little later, so the line is
            // awaited before the record is read.
            assert_eq!(rig.attempt_told().await.result, UnlockResult::KeyLost);
            assert!(matches!(
                rig.sharing.app.unlock_status(&rig.viewing.id()).unwrap(),
                UnlockStatus::Ended { .. }
            ));
            assert_eq!(rig.questions(&screen), 1);
        } else {
            rig.question(&screen, 2).await;
        }
        rig.ends(&screen).await;
    }
}

#[tokio::test]
async fn a_question_that_the_platform_does_not_take_is_declined_without_asking_it_to_cancel() {
    for code in [-1, 5, i32::MIN] {
        let mut rig = Rig::unlockable().await;
        rig.sets_up().await;
        let screen = rig.screen_shown().await;
        rig.private_in_place(&screen);
        rig.viewer.plan_open_question(code);
        rig.locks(&screen).await;
        assert!(rig.taps(&screen));
        let asked = rig.question(&screen, 1).await;
        assert_eq!(
            rig.told(&screen).await,
            [
                WindowUnlock::Asking,
                WindowUnlock::Ended(UnlockResult::GaveUp)
            ],
            "unlock_open returned {code}"
        );
        // Nobody holds the question: an answer that comes anyway is late, and the platform that
        // did not take the question is not told that it is not open.
        assert_eq!(
            rig.opened(&screen, &asked, &[9; 32]),
            Err(Error::UnknownLink)
        );
        assert_eq!(
            rig.declined(&screen, &asked, FACE_DECLINED),
            Err(Error::UnknownLink)
        );
        assert_eq!(rig.cancels(&screen), 0);
        assert_eq!(unlocks_of(&rig, &screen), []);
        rig.ends(&screen).await;
    }
}

#[tokio::test]
async fn a_mac_that_is_unlocked_while_a_face_is_asked_for_closes_the_question_once() {
    let mut rig = Rig::unlockable().await;
    rig.sets_up().await;
    let screen = rig.screen_shown().await;
    rig.private_in_place(&screen);
    rig.locks(&screen).await;
    assert!(rig.taps(&screen));
    let asked = rig.question(&screen, 1).await;

    // The person at the Mac unlocks the screen meanwhile. The person at the other device is told
    // that there was nothing to unlock, before the pictures go on.
    resume(&rig.registry, screen.link).unwrap();
    assert_eq!(
        rig.told(&screen).await,
        [
            WindowUnlock::Asking,
            WindowUnlock::Ended(UnlockResult::NotLocked)
        ]
    );
    rig.until_locked(&screen, false).await;
    // The platform that holds the question is told that it is not open, and an answer that
    // comes anyway is late.
    until(
        "the platform to be told that the question is closed",
        || rig.cancels(&screen) == 1,
    )
    .await;
    assert_eq!(
        rig.opened(&screen, &asked, &[9; 32]),
        Err(Error::UnknownLink)
    );
    assert_eq!(unlocks_of(&rig, &screen), []);
    assert_eq!(rig.cancels(&screen), 1);
    rig.ends(&screen).await;
}

#[tokio::test]
async fn a_window_that_ends_while_a_face_is_asked_for_leaves_no_question_behind() {
    let mut rig = Rig::unlockable().await;
    rig.sets_up().await;
    let screen = rig.screen_shown().await;
    rig.private_in_place(&screen);
    rig.locks(&screen).await;
    assert!(rig.taps(&screen));
    let asked = rig.question(&screen, 1).await;

    rig.ends(&screen).await;
    // The platform learns that the screen is over from the screen, and is not told on top of it
    // that its question is closed. An answer that comes anyway is late.
    assert_eq!(
        rig.opened(&screen, &asked, &[9; 32]),
        Err(Error::UnknownLink)
    );
    assert_eq!(
        rig.declined(&screen, &asked, FACE_DECLINED),
        Err(Error::UnknownLink)
    );
    assert_eq!(rig.cancels(&screen), 0);
    assert_eq!(unlocks_of(&rig, &screen), []);
}

#[tokio::test]
async fn a_platform_without_the_unlock_functions_neither_can_be_unlocked_nor_unlocks() {
    // A Mac whose platform was written before another device could unlock it. It is not asked
    // for a password, and the other device is not asked for a key.
    let rig = Rig::unlockable_with(|platform, _| platform.set_cannot_unlock()).await;
    assert!(!rig.sharing.app.window_abilities().can_be_unlocked);
    let error = rig
        .sharing
        .app
        .set_up_unlock(&rig.viewing.id(), PASSWORD.to_owned())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Other);
    assert_eq!(
        error.message(),
        "This device cannot be unlocked from another device."
    );
    let asked_for_password = |call: &SharerCall| {
        matches!(
            call,
            SharerCall::UnlockCheck { .. } | SharerCall::UnlockSetup { .. }
        )
    };
    assert_eq!(rig.platform.count(asked_for_password), 0);
    assert!(
        rig.viewer
            .log()
            .iter()
            .all(|(_, call)| !matches!(call, Call::UnlockKey { .. }))
    );

    // A device whose platform was written before it could unlock: the Mac checks the password,
    // and the other device is not asked for a key.
    let rig = Rig::unlockable_with(|_, viewer| viewer.has_no_face()).await;
    assert!(rig.sharing.app.window_abilities().can_be_unlocked);
    assert!(!rig.viewing.app.window_abilities().can_unlock);
    let error = rig
        .sharing
        .app
        .set_up_unlock(&rig.viewing.id(), PASSWORD.to_owned())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Declined);
    assert_eq!(
        error.message(),
        "Kitchen iPad cannot unlock this device. It may need a newer version of Baton."
    );
    assert_eq!(rig.platform.count(asked_for_password), 1);
    assert!(
        rig.viewer
            .log()
            .iter()
            .all(|(_, call)| !matches!(call, Call::UnlockKey { .. }))
    );
    assert_eq!(
        rig.sharing.app.unlock_status(&rig.viewing.id()).unwrap(),
        UnlockStatus::Off
    );
}

#[tokio::test]
async fn a_screen_that_the_other_device_only_looks_at_has_no_unlock() {
    let rig = Rig::unlockable().await;
    rig.sets_up().await;
    let started = rig
        .viewing
        .app
        .start_screen(&rig.sharing.id(), false)
        .await
        .unwrap();
    let ScreenStarted::Started(session) = started else {
        panic!("the screen was not started: {started:?}");
    };
    // Looking is not private: the other device could not tell a locked screen from a hidden one.
    assert!(!session.unlock, "{session:?}");
    assert!(!rig.viewing.app.unlock_window(session.id));
    let window = rig.platform.windows_made() as u64;
    until("the platform to be started", || {
        rig.platform
            .calls(window)
            .iter()
            .any(|call| matches!(call, SharerCall::Start { .. }))
    })
    .await;
    assert!(rig.viewing.app.end_window(session.id));
    rig.nothing_is_left().await;
    assert!(
        rig.viewer
            .log()
            .iter()
            .all(|(_, call)| !matches!(call, Call::UnlockOpen { .. }))
    );
}
