// SPDX-License-Identifier: Apache-2.0

//! Baton at the login window of a Mac (`docs/adr/0019-baton-at-the-login-window.md`,
//! `docs/adr/0020-screen-sharing-at-the-login-window.md`).
//!
//! A Mac that restarted with `FileVault` on stops at its login window after the phone has given it
//! the password of the disk, and the Baton app does not run there: it is a login item of one
//! account. This is a small Baton of its own that launchd starts for the login window, so that a
//! paired phone can look at the login window and use it, and sign the account in.
//!
//! It is the same Rust app layer as the app has ([`baton_app`]), with the choices that make it
//! small:
//!
//! - It is a device of its own, with a key and a list of paired devices in a folder that only
//!   its owner can read ([`State`]). It shares nothing with the app.
//! - It is paired in Terminal, at the Mac ([`pair()`]): it offers a ticket, and the person types
//!   the six digits that the phone shows. The program does not show them, and shows nothing that
//!   the device sent with the question but the system it says it runs: a stranger who holds the
//!   ticket could put the digits in its name. Pairing sets what the device may do here to the
//!   main screen with control, and nothing else changes it.
//! - It takes no links, no text, no files, reads nothing that a person works on and unlocks
//!   nothing ([`Program`]). It asks nobody anything: nobody is signed in to answer.
//! - It needs a platform ([`baton_app::Windows`]) that gives the main display of the login
//!   window and posts the events of the phone there. ADR 0020 has this platform speak Apple's own
//!   Screen Sharing as a client of it ([`screen_sharing`]), rather than capture the display
//!   itself, and check who is signed in to the Mac's own console ([`console_session`]) rather than
//!   rely on a session the system ends at sign-in. Wiring the two into a [`baton_app::Windows`]
//!   that `serve` can be given is not done here: ADR 0020's own V1 and V2 need an answer from the
//!   real Mac first, since they decide how the two fit together.
//! - The screen is meant to be given as it is, never covered or held from whoever is at the
//!   Mac, since nothing private is on a login window and a hold there could lock a person out
//!   of their own sign-in rather than someone else's. That is not yet a promise this crate can
//!   make: the app layer it shares with every other Baton asks a platform for a private screen
//!   whenever the device it answers is given control, and a login window needs the phone to
//!   have control to type a password at all. [`Program::start`]'s own doc, and
//!   `docs/adr/0020-screen-sharing-at-the-login-window.md`'s V9, say what is still open about
//!   it; nothing here answers it, since there is no platform here yet to answer it with.
//! - It logs that a session began and that one ended, and nothing more.
//!
//! What the program does is what its code promises, and nothing in the system holds it to the
//! list above if the code is wrong. The record says what that means.

// Not wired into a `Windows` yet: ADR 0020's own V1 and V2 need an answer from a real Mac before
// anything decides how the two fit together (see the module doc above).
#[allow(dead_code)]
mod console_session;
pub mod devices;
mod error;
mod name;
mod pair;
mod person;
#[allow(dead_code)]
mod screen_sharing;
mod serve;
mod state;

pub use error::{Error, ErrorKind, Result};
pub use name::name_this_mac;
pub use pair::pair;
pub use person::{Person, Terminal};
pub use serve::{Program, serve};
pub use state::State;
