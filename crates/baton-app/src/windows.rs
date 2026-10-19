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
    sync::{Arc, Mutex, Weak},
};

use baton_core::{
    streaming::{Decoder, SourceEvents, SourceHandle, Takes, ViewHandle, WindowOffer},
    util::BoxFuture,
};
use baton_proto::stream::Codec;
use baton_stream::control::Config as Format;

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

    /// The person said yes to `window`: makes the decoder and the screen for it.
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

/// The part of a platform that shares the windows of this device.
pub trait WindowSharer: Send + Sync + 'static {
    /// Whether the person can let another device control a window that they share. A platform
    /// that cannot post input says no, and no window is offered with control.
    fn control(&self) -> bool;

    /// Lets the person pick the window to share, and gives it. `None` means that the person gave
    /// up. The future ends when the person has chosen, and may take as long as that takes.
    fn pick(&self) -> BoxFuture<'static, Result<Option<Box<dyn PickedWindow>>, PlatformError>>;
}

/// A window that the person picked, and everything that the platform keeps for it. Dropping it
/// lets go of all of that.
pub trait PickedWindow: Send + 'static {
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
    /// dropping the last copy of `handle`. The app keeps no copy of its own, so a platform that
    /// lets go of the handle, because the window closed or the capture could not begin, has
    /// ended the session.
    fn start(self: Box<Self>, handle: Arc<SourceHandle>);
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
