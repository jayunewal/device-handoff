// SPDX-License-Identifier: Apache-2.0

//! What the operating system provides for showing a window of another device here, and for
//! sharing a window of this device with another one.
//!
//! A user interface does not draw a picture or capture a window: the platform does, with its
//! own frameworks (Swift on an Apple device), and it is given to the app when it starts, as
//! [`Windows`]. A device that is given nothing takes no part in window streaming and does not
//! say that it does. A device can show windows, share them, or both.
//!
//! - **Showing.** A [`WindowViewer`] says what the device can show. When the person says yes
//!   to a window that another device offered, it makes the decoder and the screen for it
//!   ([`Screen`]). The pictures go from the network to the decoder without passing through the
//!   user interface, and the interface is given only the number of the screen to draw.
//! - **Sharing.** A [`WindowSharer`] lets the person pick the window to share. What it picked
//!   ([`PickedWindow`]) takes part in the offer: it hears what the other device says, chooses
//!   the picture within what that device can take, and, when the session runs, captures and
//!   encodes the window and hands the pictures to the session.
//!
//! The calls into these traits come from the threads of the app's runtime. None of them may
//! wait for long, and a platform that has to ask the person, or the system, does so with a
//! future or from a thread of its own.

use std::{
    fmt,
    pin::pin,
    sync::{Arc, Mutex, Weak},
    task::{Context, Waker},
};

use baton_core::{
    streaming::{
        DecodeError, Decoder, DecoderState, SourceEvents, SourceHandle, Takes, ViewHandle, Viewing,
        WindowOffer,
    },
    util::BoxFuture,
};
use baton_proto::stream::Codec;
use baton_stream::{
    control::{Config as Format, SourceMessage, Thermal},
    header::FrameHeader,
};

use crate::app::locked;

/// What went wrong in the platform, in a sentence that the person can be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformError {
    message: String,
}

impl PlatformError {
    /// An error with this sentence. It is cleaned before a person sees it: it is cut to one line.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The sentence.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PlatformError {}

/// What a device can show: the largest picture, the highest rate and the codecs. The source of a
/// window keeps within it, and a source that does not has broken the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShowLimits {
    /// The widest picture in pixels, from 1 to 3840.
    pub max_width: u32,
    /// The highest picture in pixels, from 1 to 3840.
    pub max_height: u32,
    /// The most frames a second, from 1 to 120.
    pub max_fps: u16,
    /// The codecs that the decoder takes: from 1 to 8, none of them [`Codec::Other`].
    pub codecs: Vec<Codec>,
}

impl ShowLimits {
    /// Whether the numbers are within what the protocol allows: each edge from 1 to 3840 pixels,
    /// the rate from 1 to 120, and from 1 to 8 codecs, none of them [`Codec::Other`].
    ///
    /// The app checks them when it starts, and a platform that registers through the C interface
    /// has them checked when it registers, so that a platform that is set up wrongly fails at
    /// once and not at the first window.
    ///
    /// # Errors
    ///
    /// A number is outside what the protocol allows. The error says which.
    pub fn check(&self) -> Result<(), PlatformError> {
        Viewing::new(
            Unused,
            self.max_width,
            self.max_height,
            self.max_fps,
            self.codecs.clone(),
        )
        .map(|_| ())
        .map_err(|error| PlatformError::new(error.to_string()))
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

/// The decoder and the screen of a window that is shown.
pub struct Screen {
    /// Takes the pictures in the order in which they are to be decoded, and shows them on the
    /// screen. It is given what the source says too, and is told when the session is over.
    pub decoder: Box<dyn Decoder>,
    /// The number that the user interface draws the screen with: on Flutter, the id of the
    /// texture.
    pub id: i64,
}

impl fmt::Debug for Screen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Screen")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// How the screen of a window that is shown reaches its session once the session runs.
///
/// [`WindowViewer::open`] gives it to the platform, which keeps it for as long as the screen
/// lives. The session begins after `open` returns, so a link does nothing until then, and
/// nothing again once the session is over. It holds the session loosely: keeping a link does not
/// keep the session going.
#[derive(Clone, Default)]
pub struct ScreenLink {
    session: Arc<Mutex<Weak<ViewHandle>>>,
}

impl ScreenLink {
    /// The session runs: the link reaches it from now on.
    pub(crate) fn connect(&self, session: &Arc<ViewHandle>) {
        *locked(&self.session) = Arc::downgrade(session);
    }

    /// The decoder failed on a frame after it had taken it. Nothing can be decoded until a key
    /// frame comes, and the viewer asks the source for one.
    ///
    /// A decoder that fails while it is called says so with its answer instead. This is for the
    /// failure that comes later, from a thread of the platform: never from within a call of the
    /// [`Decoder`], which the session makes with its lock held.
    pub fn decode_failed(&self) {
        let session = locked(&self.session).upgrade();
        if let Some(session) = session {
            session.decode_failed();
        }
    }
}

impl fmt::Debug for ScreenLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScreenLink").finish_non_exhaustive()
    }
}

/// The part of a platform that shows the windows that other devices offer.
pub trait WindowViewer: Send + Sync + 'static {
    /// What the device can show. The same each time.
    fn limits(&self) -> ShowLimits;

    /// The person said yes to `window`, or asked for a screen and the other device is about to
    /// offer it: makes the decoder and the screen for it. In the second case `window` is a display
    /// with no title and no application, its size is not known yet (`width` and `height` are
    /// nought, and the size of the picture comes with the first picture), and `control` is what
    /// the person wished for, which the other device may not grant.
    ///
    /// The call returns at once. A platform that has to prepare something first fails here, with
    /// a sentence for the person, and the window is refused. The platform keeps the `link` for
    /// the screen.
    ///
    /// # Errors
    ///
    /// The platform cannot show the window now.
    fn open(&self, window: &WindowOffer, link: ScreenLink) -> Result<Screen, PlatformError>;
}

/// What is shared is called in a sentence of the app: a whole display is a screen.
pub(crate) fn noun(display: bool) -> &'static str {
    if display { "screen" } else { "window" }
}

/// What the person at a device is promised while another device uses its screen
/// ([`WindowSharer::screen`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Privacy {
    /// The screen is given as it is: whoever is at the device can see it, as they could before.
    Open,
    /// Another device controls the screen, and nobody at this device may see what it does or use
    /// the device meanwhile. For as long as the session lasts the platform keeps the screen dark
    /// and the keyboard and the pointer of this device out of anyone's hands, and when the session
    /// ends, or the connection is lost, it locks the device again.
    ///
    /// A platform that cannot promise that refuses the screen ([`ScreenError::Unsupported`] or
    /// [`ScreenError::NeedsInputPermission`]) and never gives it open in its place.
    Private,
}

/// Why a platform did not give the screen that it was asked for ([`WindowSharer::screen`]). The
/// device that asked is told which of these it was, so that the person there knows what to do.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScreenError {
    /// The platform does not give its screen unless the person picks it, or does not give it with
    /// the [`Privacy`] that was asked for.
    Unsupported,
    /// The operating system has not let the app capture the screen, and the person at this device
    /// has to allow it.
    NeedsPermission,
    /// The screen was asked for as [`Privacy::Private`] and the operating system has not let the
    /// app hold the keyboard and the pointer of this device, which that needs. The person at this
    /// device has to allow it.
    NeedsInputPermission,
    /// There is no screen to give now, for example because no display is on.
    Unavailable,
    /// The platform could not, and the sentence says why. The device that asked is told only that
    /// it failed, and the log of this device has the kind of the reason and not the sentence.
    Failed(PlatformError),
}

impl fmt::Display for ScreenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("the platform does not give its screen unasked"),
            Self::NeedsPermission => {
                f.write_str("the operating system has not let the app capture the screen")
            }
            Self::NeedsInputPermission => f.write_str(
                "the operating system has not let the app hold the keyboard and the pointer",
            ),
            Self::Unavailable => f.write_str("there is no screen to give now"),
            Self::Failed(error) => write!(f, "the platform could not give its screen: {error}"),
        }
    }
}

impl std::error::Error for ScreenError {}

/// The part of a platform that shares the windows of this device.
pub trait WindowSharer: Send + Sync + 'static {
    /// Whether the person can let another device control a window that they share. A platform
    /// that cannot post input says no, and no window is offered with control.
    fn control(&self) -> bool;

    /// Lets the person pick the window to share, and gives it. `None` means that the person gave
    /// up. The future ends when the person has chosen, and may take as long as that takes.
    fn pick(&self) -> BoxFuture<'static, Result<Option<Box<dyn PickedWindow>>, PlatformError>>;

    /// Whether the platform can give the screen of this device without the person picking it
    /// ([`screen`](Self::screen)). A device where it can lets the devices it is paired with ask it
    /// to start sharing its screen, if the person here allowed them to
    /// ([`App::set_start_access`](crate::App::set_start_access)), and says so to them. A device
    /// where it cannot says nothing of the kind. The same each time.
    fn can_give_screen(&self) -> bool {
        false
    }

    /// Gives the main screen of this device, with every window on it, without the person picking
    /// it: a paired device asked for it, and the person here allowed that device beforehand. What
    /// is given must be a whole display ([`PickedWindow::display`]); anything else is refused.
    ///
    /// `privacy` is what the person here is promised while the screen is used. With
    /// [`Privacy::Private`] the platform checks beforehand that it can keep the promise and
    /// refuses if it cannot; it keeps it from the time the session starts
    /// ([`PickedWindow::start`]) until the screen is let go of, and a screen that is dropped
    /// without ever having started changes nothing at the device.
    ///
    /// The future may take as long as the system takes. It is dropped when the device that asked
    /// gives up, or the time for the answer is up, and whatever it was about to give is let go of
    /// then. Only called if [`can_give_screen`](Self::can_give_screen) says yes.
    fn screen(
        &self,
        privacy: Privacy,
    ) -> BoxFuture<'static, Result<Box<dyn PickedWindow>, ScreenError>> {
        let _ = privacy;
        Box::pin(std::future::ready(Err(ScreenError::Unsupported)))
    }

    /// Whether the device that controls a screen that this platform gave as [`Privacy::Private`]
    /// can lift what keeps it private, in the part that it likes and as often as it likes: the
    /// screen shown to the people here, and the keyboard and the pointer of this device put to
    /// use. The platform hears what the device says through [`SourceEvents::lift`]. A device
    /// where it can says so to the devices it is paired with; one where it cannot gives its
    /// private screen as it did, with nothing for the device that controls it to lift. The same
    /// each time.
    fn can_lift(&self) -> bool {
        false
    }
}

/// A window that the person picked, and everything that the platform keeps for it. Dropping it
/// lets go of all of that.
pub trait PickedWindow: Send + 'static {
    /// Whether what the person picked is a whole display, with every window on it, and not one
    /// window. The size is then that of the display, and the title is its name, if the platform
    /// knows one. A platform that cannot share a display leaves this as it is.
    fn display(&self) -> bool {
        false
    }

    /// Whether the screen was given as [`Privacy::Private`] by a platform whose sharer
    /// [`can_lift`](WindowSharer::can_lift). The device that controls it is told, and can lift
    /// what keeps it private; for a screen that is not, it can lift nothing. A platform that
    /// cannot lift leaves this as it is.
    fn private(&self) -> bool {
        false
    }

    /// The title of the window, which the person at the other device is shown. May be empty.
    fn title(&self) -> String;

    /// The name of the application that the window belongs to. May be empty.
    fn app(&self) -> String;

    /// The size of the window in pixels.
    fn size(&self) -> (u32, u32);

    /// Takes what the other device says during the session: how it is doing, what it can take,
    /// what the person does there (only if the window was offered with control), and the end. It
    /// is called once, before the window is offered.
    ///
    /// Every call is made with the session's lock held, so none of them waits and none of them
    /// calls the [`SourceHandle`]: a platform that takes time hands the work to a thread of its
    /// own.
    fn events(&mut self) -> Box<dyn SourceEvents>;

    /// The other device said yes, and says what it can take. Chooses the picture to begin with,
    /// within that, and gets ready to make it, or says that no picture within it can be made
    /// (`None`), which ends the session at once. The format's `epoch` is ignored.
    fn prepare(&mut self, takes: &Takes) -> Option<Format>;

    /// The session runs. The platform captures and encodes the window now: before it encodes a
    /// picture it asks `handle` whether to ([`SourceHandle::begin`]), and it hands over what it
    /// encoded ([`SourceHandle::sent`]). The session ends when the other device ends it, when
    /// the connection ends, and when the platform ends it, with [`SourceHandle::end`] or by
    /// dropping the last copy of `handle`, which the person is not told of: they closed the
    /// window, or ended the session themselves. The app keeps no copy of its own, so a platform
    /// that lets go of the handle has ended the session.
    ///
    /// A platform that has to end the session because something went wrong, such as a capture
    /// that cannot begin or that the system stopped, says so with
    /// [`failure`](SharingFailure::fail) instead, and the person is shown the sentence.
    fn start(self: Box<Self>, handle: Arc<SourceHandle>, failure: SharingFailure);
}

/// How a platform ends a window that this device shares because something went wrong, and says
/// why. [`PickedWindow::start`] gives it with the handle.
///
/// It holds the session loosely, as [`ScreenLink`] does: keeping a failure does not keep the
/// session going.
#[derive(Clone)]
pub struct SharingFailure {
    note: Arc<Note>,
}

struct Note {
    message: Mutex<Option<String>>,
    handle: Weak<SourceHandle>,
}

impl SharingFailure {
    pub(crate) fn new(handle: &Arc<SourceHandle>) -> Self {
        Self {
            note: Arc::new(Note {
                message: Mutex::new(None),
                handle: Arc::downgrade(handle),
            }),
        }
    }

    /// Ends the session, and tells the person why, in a sentence about what happened that they
    /// can act on. The first sentence is the one that counts, and a sentence with nothing in it
    /// ends the session without one. Does nothing to a session that is over, whose end is told
    /// as it came.
    ///
    /// The platform calls this instead of letting go of the handle: a session that is ended by
    /// the handle first is told as the end of a window that the person closed.
    pub fn fail(&self, message: impl Into<String>) {
        let Some(handle) = self.note.handle.upgrade() else {
            return;
        };
        // A session that is over ended as it ended, and a sentence that comes after that is not
        // the reason: a thread of the platform that has not heard that the person closed the
        // window yet does not turn the end into a failure.
        let mut over = pin!(handle.on_end());
        if over
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready()
        {
            return;
        }
        {
            let mut note = locked(&self.note.message);
            if note.is_none() {
                *note = Some(message.into());
            }
        }
        handle.end();
    }

    /// The sentence that the platform gave, if it gave one.
    pub(crate) fn message(&self) -> Option<String> {
        locked(&self.note.message).clone()
    }
}

impl fmt::Debug for SharingFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharingFailure").finish_non_exhaustive()
    }
}

/// What a device is given to take part in window streaming with. The default is nothing.
#[derive(Clone, Default)]
pub struct Windows {
    pub(crate) viewer: Option<Arc<dyn WindowViewer>>,
    pub(crate) sharer: Option<Arc<dyn WindowSharer>>,
}

impl Windows {
    /// A device that takes no part.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// The same, with a platform that shows the windows of other devices.
    #[must_use]
    pub fn showing(mut self, viewer: Arc<dyn WindowViewer>) -> Self {
        self.viewer = Some(viewer);
        self
    }

    /// The same, with a platform that shares windows of this device.
    #[must_use]
    pub fn sharing(mut self, sharer: Arc<dyn WindowSharer>) -> Self {
        self.sharer = Some(sharer);
        self
    }
}

impl fmt::Debug for Windows {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Windows")
            .field("shows", &self.viewer.is_some())
            .field("shares", &self.sharer.is_some())
            .finish()
    }
}
