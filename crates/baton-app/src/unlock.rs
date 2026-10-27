// SPDX-License-Identifier: Apache-2.0

//! Unlocking a locked screen from another device (`docs/adr/0013-unlock-from-a-phone.md`): what
//! the platform of each device provides, and what the app does with it.
//!
//! The device that is unlocked (a Mac) keeps the password of its person in a box that only the
//! other device (an iPhone or an iPad) can open, after the face of its person is recognised. The
//! person sets it up at the Mac with [`App::set_up_unlock`](crate::App::set_up_unlock): the
//! platform checks the password against the account ([`WindowSharer::check_password`]), the other
//! device is asked for a public key ([`WindowViewer::unlock_key`]), and the platform puts the
//! password in a box and seals the key of the box to that key ([`WindowSharer::unlock_setup`]).
//! When the screen is locked, the person taps Unlock at the other device
//! ([`App::unlock_window`](crate::App::unlock_window)), the platform there asks for their face
//! ([`FaceUnlocker`]), and the platform of the Mac types the password
//! ([`SourceEvents::unlock`](baton_core::streaming::SourceEvents::unlock)).
//!
//! The password, the keys and the box are secrets, and none of the types here prints them.

use std::{fmt, sync::Arc};

use baton_core::{
    NewUnlock, PairedDevice, TrustStore,
    clock::{Clock as _, SystemClock},
    error::Error as Core,
    streaming::{CancelReason, KeyOutcome, Question, Settled, UnlockApprover, Unlocker},
    util::BoxFuture,
};
use baton_proto::{
    DeviceId,
    message::DeclineReason,
    unlock::{BoxKey, SealedKey, UnlockKey},
};
use tokio::sync::broadcast;

use crate::{
    Error, ErrorKind, Event, Result,
    model::{UnlockEntry, UnlockResult, UnlockSetUp, WindowUnlock},
    sessions::Pictures,
    windows::{PlatformError, WindowSharer, WindowViewer},
};

/// The password of the person at this device, which they typed to set the unlock up. It is wiped
/// from memory when it is dropped and never printed. A platform reads it with
/// [`expose`](Self::expose), copies as little of it as it can, and wipes what it copies.
pub struct Password(zeroize::Zeroizing<String>);

impl Password {
    /// The most bytes that a password can have and be kept. The box that holds it is padded to 128
    /// bytes, and one of them says how long the password is.
    pub const MAX_BYTES: usize = 127;

    pub(crate) fn new(text: zeroize::Zeroizing<String>) -> Self {
        Self(text)
    }

    /// The password.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(..)")
    }
}

/// The paired device that an unlock is for, as the platform of the device that unlocks it is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnlockSource {
    /// Its id. The key that the platform makes is the key for this device and for no other.
    pub id: DeviceId,
    /// The name to show for it: the name the person gave it, or else the name it calls itself.
    /// A prompt that asks for the face names the device with it.
    pub name: String,
}

impl UnlockSource {
    pub(crate) fn of(device: &PairedDevice) -> Self {
        Self {
            id: device.id,
            name: device.display_name().to_owned(),
        }
    }
}

/// What the platform of the device that is unlocked is given to keep the password in a box that
/// only the other device opens ([`WindowSharer::unlock_setup`]).
///
/// The platform puts the password in a box under a random key, padded to 128 bytes and
/// authenticated together with a context string and the two ids, seals that key to
/// [`key`](Self::key), wipes the password and the key, and gives the box and the sealed key back.
pub struct UnlockSetup {
    /// The password, which the platform has accepted as the one of the person's account.
    pub password: Password,
    /// This device. The box is bound to it, so a box taken from another device opens nothing here.
    pub target: DeviceId,
    /// The paired device that unlocks this one. The box is bound to it as well.
    pub viewer: DeviceId,
    /// The key of that device that the key of the box is sealed to, now and at every unlock.
    pub key: UnlockKey,
}

impl fmt::Debug for UnlockSetup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnlockSetup")
            .field("password", &self.password)
            .field("target", &self.target.short())
            .field("viewer", &self.viewer.short())
            .field("key", &self.key)
            .finish()
    }
}

/// Why a platform did not accept a password or did not make the box
/// ([`WindowSharer::check_password`], [`WindowSharer::unlock_setup`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnlockSetupError {
    /// The system here does not accept the password as the one of the account of the person.
    WrongPassword,
    /// This device cannot keep an unlock after all: it cannot check a password against the
    /// account, or its lock screen does not take one.
    Unsupported,
    /// The platform could not, and the sentence says why. It is shown to the person.
    Failed(PlatformError),
}

/// Why a platform did not give the key that an unlock is sealed to
/// ([`WindowViewer::unlock_key`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnlockKeyError {
    /// The person here said no, when the platform asked whether the device may be unlocked with
    /// their face.
    Refused,
    /// The settings of this device do not let that device ask.
    Blocked,
    /// This device cannot make such a key: it has no hardware to keep one in, or no face that it
    /// recognises.
    Unsupported,
    /// The platform could not, for a reason of its own. The log of this device has the sentence,
    /// and the device that asked is told that this one cannot, as for
    /// [`Unsupported`](Self::Unsupported): the protocol has no word for a failure.
    Failed(PlatformError),
}

/// What asks for the face of the person here when a device that shares a screen asks a question
/// to unlock it ([`Screen::with_unlocker`](crate::windows::Screen::with_unlocker)).
///
/// The person taps Unlock ([`App::unlock_window`](crate::App::unlock_window)), the device that
/// shares the screen sends its question, and the platform asks for their face, in words that
/// name that device, and answers when it can. Every method is called with the lock of the
/// session held, so none of them waits and none answers a question before it returns: a platform
/// that asks for a face hands the work to a thread of its own.
pub trait FaceUnlocker: Send + 'static {
    /// The question came. The platform asks for the face of the person, and answers the question
    /// with [`UnlockQuestion::opened`] or [`UnlockQuestion::declined`].
    fn open(&mut self, question: UnlockQuestion);

    /// The attempt ended while the question was open: the device that shares the screen took it
    /// back, or the session is ending. The platform stops asking for the face. An answer that
    /// comes later is dropped.
    fn cancel(&mut self);

    /// The session is over. The platform lets go of everything it holds for it, and is called no
    /// more.
    fn ended(&mut self);
}

/// The question of the device that shares a screen, which the person here answers with their
/// face.
///
/// It is the key of the box of that device, sealed to a key that this device made for it. The
/// platform opens it with the private half of that key, which works only after the face of the
/// person is recognised, and answers with [`opened`](Self::opened). If the person does not
/// agree, their face is not recognised, or the key is gone, it answers with
/// [`declined`](Self::declined). A question that is dropped is declined with
/// [`CancelReason::Failed`].
///
/// The platform opens the sealed key with the key it made for [`source`](Self::source), and
/// tries no other: a sealed key does not name its key, and a platform that tried its keys in turn
/// would open one that was copied from another device. A platform that has no key for it
/// declines with [`CancelReason::KeyLost`].
///
/// An answer that comes after the attempt is over is dropped, and the key in it is wiped.
pub struct UnlockQuestion {
    question: Option<Question>,
    sealed: SealedKey,
    number: u64,
    source: UnlockSource,
    pictures: Arc<Pictures>,
}

impl UnlockQuestion {
    /// The key of the box of the device that shares the screen, sealed to the key that this device
    /// made for it.
    #[must_use]
    pub fn sealed(&self) -> &SealedKey {
        &self.sealed
    }

    /// The device that shares the screen: the device whose key opens the sealed key.
    #[must_use]
    pub fn source(&self) -> &UnlockSource {
        &self.source
    }

    /// The face was recognised and the sealed key is open: sends the key to the device that shares
    /// the screen. The platform wipes its own copy.
    pub fn opened(mut self, key: BoxKey) {
        let Some(question) = self.question.take() else {
            return;
        };
        // Told before the key goes, so that it is told before what the other device says of it.
        if self.pictures.answered(self.number, WindowUnlock::Typing) {
            question.opened(key);
        }
    }

    /// The person does not agree, their face was not recognised, or the key is gone.
    pub fn declined(mut self, reason: CancelReason) {
        let Some(question) = self.question.take() else {
            return;
        };
        let end = WindowUnlock::Ended(UnlockResult::from(reason));
        if self.pictures.answered(self.number, end) {
            question.declined(reason);
        }
    }
}

impl Drop for UnlockQuestion {
    fn drop(&mut self) {
        // The question declines itself when it is dropped; the person is told as well.
        if self.question.take().is_some() {
            self.pictures.answered(
                self.number,
                WindowUnlock::Ended(UnlockResult::from(CancelReason::Failed)),
            );
        }
    }
}

impl fmt::Debug for UnlockQuestion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnlockQuestion")
            .field("source", &self.source.id.short())
            .finish_non_exhaustive()
    }
}

/// The unlocker of the platform, with the app hearing how an attempt goes. It passes the question
/// and the end on as they are.
pub(crate) struct Asked {
    pub(crate) unlocker: Box<dyn FaceUnlocker>,
    pub(crate) pictures: Arc<Pictures>,
    pub(crate) source: UnlockSource,
}

impl Unlocker for Asked {
    fn open(&mut self, question: Question) {
        let number = self.pictures.asking();
        self.unlocker.open(UnlockQuestion {
            sealed: question.sealed().clone(),
            question: Some(question),
            number,
            source: self.source.clone(),
            pictures: Arc::clone(&self.pictures),
        });
    }

    fn cancel(&mut self) {
        self.pictures.taken_back();
        self.unlocker.cancel();
    }

    fn settled(&mut self, settled: Settled) {
        self.pictures.settled(UnlockResult::from(settled));
    }

    fn ended(&mut self) {
        self.unlocker.ended();
    }
}

/// What a paired device is answered when it asks this one for the key an unlock is sealed to: the
/// platform makes it.
pub(crate) struct Approver(pub(crate) Arc<dyn WindowViewer>);

impl UnlockApprover for Approver {
    fn key<'a>(
        &'a self,
        source: &'a PairedDevice,
        renew: bool,
    ) -> BoxFuture<'a, std::result::Result<UnlockKey, DeclineReason>> {
        let made = self.0.unlock_key(UnlockSource::of(source), renew);
        Box::pin(async move {
            made.await.map_err(|error| match error {
                UnlockKeyError::Refused => DeclineReason::Refused,
                UnlockKeyError::Blocked => DeclineReason::Blocked,
                UnlockKeyError::Unsupported => DeclineReason::Unsupported,
                UnlockKeyError::Failed(error) => {
                    tracing::warn!(%error, "the platform could not make a key for an unlock");
                    DeclineReason::Other
                }
            })
        })
    }
}

/// The attempts of `now` that `seen` did not have. Both are oldest first. The oldest attempts drop
/// out when there are too many, so what was seen can start further on in what is there now.
///
/// Two attempts that are equal cannot be told apart, and attempts are seconds apart.
fn fresh<'a, T: PartialEq>(seen: &[T], now: &'a [T]) -> &'a [T] {
    // The most of what was seen that is still there at the start. If none of it is, everything
    // is new.
    let kept = (0..seen.len())
        .map(|dropped| &seen[dropped..])
        .find(|kept| now.starts_with(kept))
        .map_or(0, <[T]>::len);
    &now[kept..]
}

/// The attempts to unlock this device, oldest first.
fn history(trust: &TrustStore) -> Vec<baton_core::UnlockEntry> {
    let mut attempts = trust.unlock_history();
    attempts.reverse();
    attempts
}

/// The attempts to unlock this device that the person has been told of, so that each is told once
/// when it is written to the history.
pub(crate) struct Attempts {
    seen: Vec<baton_core::UnlockEntry>,
}

impl Attempts {
    /// The attempts in the history now are the ones that were told of.
    pub(crate) fn new(trust: &TrustStore) -> Self {
        Self {
            seen: history(trust),
        }
    }

    /// Tells of each attempt that was written to the history since the last call.
    pub(crate) fn announce(&mut self, trust: &TrustStore, events: &broadcast::Sender<Event>) {
        let now = history(trust);
        for entry in fresh(&self.seen, &now) {
            // Nobody listening is fine.
            let _ = events.send(Event::UnlockAttempted(UnlockEntry::from(entry)));
        }
        self.seen = now;
    }
}

/// What it takes to set the unlock up for one paired device.
pub(crate) struct SetUp<'a> {
    pub(crate) node: &'a baton_core::Node,
    pub(crate) trust: &'a TrustStore,
    pub(crate) sharer: &'a Arc<dyn WindowSharer>,
    /// This device.
    pub(crate) me: DeviceId,
    /// The paired device that is to unlock it.
    pub(crate) peer: DeviceId,
    /// The name to call that device by.
    pub(crate) name: &'a str,
}

impl SetUp<'_> {
    /// Sets the unlock up with the password `text`, in the order that spares the person the most:
    /// the password is checked first, and the other device is asked for a key only for one that
    /// this device accepts, since a new key ends what was sealed to the old one.
    pub(crate) async fn run(self, text: zeroize::Zeroizing<String>) -> Result<UnlockSetUp> {
        let name = self.name;
        if text.is_empty() {
            return Ok(UnlockSetUp::NotDone(
                "Type the password of this device.".to_owned(),
            ));
        }
        if text.len() > Password::MAX_BYTES {
            return Ok(UnlockSetUp::NotDone(format!(
                "That password is too long for Baton to keep for an unlock. It can be {} bytes at \
                 most, and a letter with an accent or from another alphabet takes two or more.",
                Password::MAX_BYTES
            )));
        }
        let password = Password::new(text);

        let sharer = Arc::clone(self.sharer);
        let checked = tokio::task::spawn_blocking(move || {
            sharer.check_password(&password).map(|()| password)
        })
        .await
        .map_err(|_| platform_failed())?;
        let password = match checked {
            Ok(password) => password,
            Err(error) => return Ok(not_done(error)),
        };

        // The other device makes a new pair for this one, and what was sealed to the old pair opens
        // nothing from then on (section 9 of the protocol).
        let key = match self.node.unlock_key(&self.peer, true).await {
            Ok(KeyOutcome::Given(key)) => key,
            Ok(KeyOutcome::Declined(reason)) => {
                return Ok(UnlockSetUp::NotDone(key_declined(name, reason)));
            }
            // An outcome that this version does not know.
            Ok(_) => {
                return Ok(UnlockSetUp::NotDone(key_declined(
                    name,
                    DeclineReason::Other,
                )));
            }
            Err(error) => return Err(Error::keying(&error, name)),
        };

        // The other device has made its new key, and what was sealed to the old one opens nothing.
        // Unless the new box is kept, the unlock that was set up before is over, whatever stops the
        // set-up from here on, and the person is told so.
        let mut renewal = Renewal::new(self.trust, self.peer);
        let (target, viewer) = (self.me, self.peer);
        let sharer = Arc::clone(self.sharer);
        let made = tokio::task::spawn_blocking(move || {
            sharer.unlock_setup(UnlockSetup {
                password,
                target,
                viewer,
                key,
            })
        })
        .await
        .map_err(|_| platform_failed())?;
        let not_kept = match made {
            Ok(new) => self.keep(new).await?,
            Err(error) => Some(why_not_made(error)),
        };
        match not_kept {
            None => {
                renewal.kept();
                Ok(UnlockSetUp::Done)
            }
            Some(sentence) => Ok(UnlockSetUp::NotDone(renewal.over(name, &sentence).await)),
        }
    }

    /// Keeps the box that the platform made. Gives nothing if it is kept, and otherwise why not,
    /// in words for the person.
    async fn keep(&self, new: NewUnlock) -> Result<Option<String>> {
        let (trust, viewer, target) = (self.trust.clone(), self.peer, self.me);
        let kept = tokio::task::spawn_blocking(move || {
            trust.set_up_unlock(&viewer, target, new, SystemClock.now_unix())
        })
        .await
        .map_err(|_| platform_failed())?;
        match kept {
            Ok(true) => {
                tracing::info!(peer = %viewer.short(), "an unlock was set up");
                Ok(None)
            }
            Ok(false) => Err(Error::not_paired()),
            // The platform made something that is beyond the limits of the record.
            Err(Core::BadItem(why)) => {
                tracing::warn!(why, "the platform made a box that cannot be kept");
                Ok(Some(
                    "This device could not make the box for the unlock.".to_owned(),
                ))
            }
            Err(error) => Err(Error::local(&error)),
        }
    }
}

/// The unlock that was set up for the other device before, from the moment that device made a new
/// key for this one. What was sealed to the old key opens nothing from then on, so unless the new
/// box is kept the record of the old unlock must not stand as set up: it would show `On` and fail
/// at the first tap. The record is turned off when this is dropped without [`kept`](Self::kept)
/// or [`over`](Self::over), so a set-up that fails on the way, and one that is given up (its
/// future is dropped), leave none.
struct Renewal<'a> {
    trust: &'a TrustStore,
    peer: DeviceId,
    settled: bool,
}

impl<'a> Renewal<'a> {
    fn new(trust: &'a TrustStore, peer: DeviceId) -> Self {
        Self {
            trust,
            peer,
            settled: false,
        }
    }

    /// The new box is kept, in place of the old record.
    fn kept(&mut self) {
        self.settled = true;
    }

    /// The set-up stopped, and `sentence` says why. Turns the old record off, and gives the
    /// sentence with what became of it added, naming the device that made the new key.
    async fn over(&mut self, device: &str, sentence: &str) -> String {
        self.settled = true;
        let (trust, peer) = (self.trust.clone(), self.peer);
        let off = tokio::task::spawn_blocking(move || trust.turn_off_unlock(&peer)).await;
        match off {
            // There was none to turn off, so there is nothing more to say.
            Ok(Ok(false)) => sentence.to_owned(),
            Ok(Ok(true)) => format!(
                "{sentence} The unlock that was set up before is turned off, because {device} made \
                 a new key for this device. Set it up again."
            ),
            _ => format!(
                "{sentence} The unlock that was set up before does not work any more, because \
                 {device} made a new key for this device, and it could not be turned off. Turn it \
                 off and set it up again."
            ),
        }
    }
}

impl Drop for Renewal<'_> {
    fn drop(&mut self) {
        if !self.settled
            && let Err(error) = self.trust.turn_off_unlock(&self.peer)
        {
            tracing::warn!(%error, "the unlock that was set up before could not be turned off");
        }
    }
}

/// What the person is told when a call into the platform did not come back: the thread it ran on
/// panicked.
fn platform_failed() -> Error {
    Error::new(
        ErrorKind::Other,
        "This device could not set the unlock up. Try again.",
    )
}

/// What came of a platform that did not accept a password or did not make the box.
fn not_done(error: UnlockSetupError) -> UnlockSetUp {
    match error {
        UnlockSetupError::WrongPassword => UnlockSetUp::WrongPassword,
        other => UnlockSetUp::NotDone(why_not_made(other)),
    }
}

/// What the person is told when the platform did not make the box. A platform that does not accept
/// the password here accepted it a moment ago, when it was checked, and the sentence says so.
fn why_not_made(error: UnlockSetupError) -> String {
    match error {
        UnlockSetupError::WrongPassword => {
            "This device accepted that password a moment ago and does not accept it now.".to_owned()
        }
        UnlockSetupError::Unsupported => "This device cannot keep an unlock: it cannot check the \
             password, or its lock does not take one."
            .to_owned(),
        UnlockSetupError::Failed(error) => Error::new(ErrorKind::Other, error.message())
            .message()
            .to_owned(),
    }
}

/// What the person is told when the other device will not give the key.
fn key_declined(device: &str, reason: DeclineReason) -> String {
    match reason {
        DeclineReason::Refused => format!("Nobody at {device} agreed to unlock this device."),
        DeclineReason::Blocked => format!(
            "{device} does not let this device ask it for a key: its settings do not allow it."
        ),
        DeclineReason::Unsupported => format!(
            "{device} cannot unlock this device. It may have no face set up for it, or need a \
             newer version of Baton."
        ),
        DeclineReason::RateLimited => {
            format!("{device} is busy with another question about a key. Try again in a moment.")
        }
        _ => format!("{device} did not give a key for the unlock."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_password_is_not_printed() {
        let password = Password::new(zeroize::Zeroizing::new("hunter2".to_owned()));
        assert_eq!(format!("{password:?}"), "Password(..)");
        let setup = UnlockSetup {
            password,
            target: DeviceId::from_bytes([1; DeviceId::LEN]),
            viewer: DeviceId::from_bytes([2; DeviceId::LEN]),
            key: UnlockKey {
                sealing: baton_proto::unlock::Sealing::EciesP256,
                public: baton_proto::unlock::SealingKey::new(vec![4; 65]),
            },
        };
        assert!(!format!("{setup:?}").contains("hunter2"));
    }

    #[test]
    fn a_password_is_read_as_it_was_typed() {
        for text in ["  spaced  ", "caf\u{e9}", "p\u{1f510}ss"] {
            assert_eq!(
                Password::new(zeroize::Zeroizing::new(text.to_owned())).expose(),
                text
            );
        }
    }

    #[test]
    fn the_attempts_that_are_new_are_those_after_what_was_seen() {
        let fresh_of = |seen: &[u8], now: &[u8]| fresh(seen, now).to_vec();
        // Nothing changed, or something else did.
        assert_eq!(fresh_of(&[], &[]), Vec::<u8>::new());
        assert_eq!(fresh_of(&[1, 2], &[1, 2]), Vec::<u8>::new());
        // One or more were added.
        assert_eq!(fresh_of(&[], &[1]), [1]);
        assert_eq!(fresh_of(&[1, 2], &[1, 2, 3]), [3]);
        assert_eq!(fresh_of(&[1, 2], &[1, 2, 3, 4]), [3, 4]);
        // The oldest dropped out as new ones came.
        assert_eq!(fresh_of(&[1, 2, 3], &[2, 3, 4]), [4]);
        assert_eq!(fresh_of(&[1, 2, 3], &[3, 4, 5]), [4, 5]);
        // Everything is new: nothing of what was seen is left.
        assert_eq!(fresh_of(&[1, 2, 3], &[4, 5, 6]), [4, 5, 6]);
        assert_eq!(fresh_of(&[1, 2, 3], &[]), Vec::<u8>::new());
    }

    #[test]
    fn what_a_device_that_will_not_give_a_key_is_told_names_it_and_says_why() {
        let said = |reason| key_declined("Anna's iPhone", reason);
        assert_eq!(
            said(DeclineReason::Refused),
            "Nobody at Anna's iPhone agreed to unlock this device."
        );
        assert_eq!(
            said(DeclineReason::Blocked),
            "Anna's iPhone does not let this device ask it for a key: its settings do not allow it."
        );
        assert_eq!(
            said(DeclineReason::Unsupported),
            "Anna's iPhone cannot unlock this device. It may have no face set up for it, or need a \
             newer version of Baton."
        );
        assert_eq!(
            said(DeclineReason::RateLimited),
            "Anna's iPhone is busy with another question about a key. Try again in a moment."
        );
        // Whatever else it said, and whatever a newer version says.
        for reason in [
            DeclineReason::TooLarge,
            DeclineReason::Invalid,
            DeclineReason::NoSpace,
            DeclineReason::Other,
        ] {
            assert_eq!(
                said(reason),
                "Anna's iPhone did not give a key for the unlock.",
                "{reason:?}"
            );
        }
    }

    #[test]
    fn a_platform_that_stops_accepting_a_password_it_accepted_says_so() {
        // Once the other device made its new key, the set-up is not one that the person can just
        // type the password for again: the sentence says what happened.
        assert_eq!(
            why_not_made(UnlockSetupError::WrongPassword),
            "This device accepted that password a moment ago and does not accept it now."
        );
        assert!(
            why_not_made(UnlockSetupError::Unsupported)
                .starts_with("This device cannot keep an unlock")
        );
    }

    #[test]
    fn a_platform_that_does_not_accept_the_password_is_not_a_failure() {
        assert_eq!(
            not_done(UnlockSetupError::WrongPassword),
            UnlockSetUp::WrongPassword
        );
        let UnlockSetUp::NotDone(sentence) = not_done(UnlockSetupError::Failed(
            PlatformError::new("The keychain is locked.\nTry later."),
        )) else {
            panic!("a failure says why");
        };
        assert!(!sentence.contains('\n'), "{sentence}");
        assert!(
            sentence.starts_with("The keychain is locked."),
            "{sentence}"
        );
    }
}
