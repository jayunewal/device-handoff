// SPDX-License-Identifier: Apache-2.0

//! Baton at the login window of a Mac (`docs/adr/0019-baton-at-the-login-window.md`).
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
//!   its owner, root, can read ([`State`]). It shares nothing with the app.
//! - It is paired in Terminal, at the Mac ([`pair()`]): it offers a ticket, and the person types
//!   the six digits that the phone shows, which the program does not show itself. Pairing sets
//!   what the device may do here to the main screen with control, and nothing else changes it.
//! - It takes no links, no text, no files, reads nothing that a person works on and unlocks
//!   nothing ([`Program`]). It asks nobody anything: nobody is signed in to answer.
//! - It needs a platform ([`baton_app::Windows`]) that gives the main display of the login
//!   window and posts the events of the phone there. On a Mac that is Swift code, registered
//!   through `baton-stream-abi` before the program starts.
//! - The screen is given as it is. The program starts the app layer with its privacy mode off
//!   ([`baton_app::Options::privacy_mode`]), so when the phone wants control the app asks the
//!   platform for the screen open ([`Privacy::Open`](baton_app::windows::Privacy::Open)), and
//!   tells the phone that the screen is not private. The app never asks this platform for a
//!   private screen, and a platform for the login window does not make one: it does not cover
//!   the display or hold the keyboard and the pointer, since nothing private is on a login
//!   window and a hold there could lock the person out. Whoever is at the Mac sees what the phone
//!   sees and can use the keys and the pointer. The record gives the reasons.
//! - It logs that a session began and that one ended, and nothing more.
//!
//! What the program does is what its code promises: it runs as root, and nothing in the system
//! holds it to the list above if the code is wrong. The record says what that means.

pub mod devices;
mod error;
mod name;
mod pair;
mod person;
mod serve;
mod state;

pub use error::{Error, ErrorKind, Result};
pub use name::name_this_mac;
pub use pair::pair;
pub use person::{Person, Terminal};
pub use serve::{Program, serve};
pub use state::State;
