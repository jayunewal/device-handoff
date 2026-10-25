// SPDX-License-Identifier: Apache-2.0

//! What a user interface calls.
//!
//! An [`App`] is one running Baton on one device. It opens the device's folder, starts the
//! network and keeps what other devices send in the inbox. A screen drives it with a few
//! calls (pair a device, send a link, read the inbox) and learns what happens on its own
//! through [`Events`].
//!
//! Nothing here belongs to a user interface toolkit. Every value that crosses this boundary
//! is plain data (see [`model`]), and every failure is an [`Error`] with a kind a screen can
//! act on and a sentence it can show. A bridge to another language, such as the one the
//! Flutter app uses, wraps this crate and adds nothing to what a device is allowed to do.
//!
//! Files go both ways. [`App::send_files`] starts a transfer and returns a [`FileSend`] to show
//! and wait for. A device takes files only if the app gives it a folder for them
//! ([`Options::receive_dir`]), and then each transfer is put to the person with
//! [`Event::TransferRequested`] before a byte is kept.
//!
//! ```no_run
//! use baton_app::{App, Event, Options};
//!
//! # async fn ask_the_person(_request: &baton_app::PairingRequest) -> bool { true }
//! # async fn demo() -> baton_app::Result<()> {
//! // A phone cannot tell its own name, so the screen asks for one first.
//! let folder = std::path::Path::new("/path/to/the/app/folder");
//! if baton_app::settings::device_name(folder)?.is_none() {
//!     baton_app::settings::set_device_name(folder, "Kitchen iPad")?;
//! }
//!
//! let app = App::start(Options::new(folder)).await?;
//! let mut events = app.events();
//! let offer = app.offer_pairing().await?;
//! println!("show a QR code of {}", offer.ticket);
//!
//! while let Some(event) = events.next().await {
//!     match event {
//!         Event::PairingRequested(request) => {
//!             // Show all of it, with the code, and say what the code is to be compared with:
//!             // the one the other device shows. Only the person can say whether they match.
//!             let same = ask_the_person(&request).await;
//!             app.answer_pairing(request.id, same);
//!         }
//!         Event::Received(item) => println!("{:?}", item),
//!         _ => {}
//!     }
//! }
//! app.stop().await;
//! # Ok(())
//! # }
//! ```

mod app;
mod error;
mod events;
mod files;
pub mod model;
mod sessions;
pub mod settings;
mod starts;
pub mod windows;

pub use app::{App, Options};
pub use error::{Error, ErrorKind};
pub use events::{Event, Events};
pub use files::FileSend;
pub use model::{
    Device, FileProgress, FilesSent, Me, Network, Offer, OfferOutcome, PairingRequest, Platform,
    PreviewName, Reach, Received, ReceivedItem, Receiving, Route, ScreenStarted, SendStage,
    StartAccess, TransferEnd, TransferRequest, TransferResult, WindowAbilities, WindowButton,
    WindowEnd, WindowInput, WindowModifiers, WindowNoticeReason, WindowPointer, WindowRequest,
    WindowRole, WindowSession, WindowShared, web_address,
};
pub use windows::Windows;

/// What the calls in this crate return.
pub type Result<T> = std::result::Result<T, Error>;
