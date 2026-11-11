// SPDX-License-Identifier: Apache-2.0

//! The program itself: a Baton that has no screen of its own, takes nothing but a request for the
//! main screen from a device that was paired with it, and runs until it is told to stop.
//!
//! What the Mac gives it is a platform ([`Windows`]) that captures the display of the login
//! window and posts the events of the phone there. On a Mac that is Swift code that registers
//! itself through `baton-stream-abi` before this program starts. Here it is whatever the caller
//! passes, so that everything else can be run, and tested, on any computer.

use std::future::Future;

use baton_app::{App, Event, Events, Options, Windows};

use crate::{Error, ErrorKind, Result, State, name::name_this_mac};

/// A Baton that is running with the folder of the program.
#[derive(Debug)]
pub struct Program {
    app: App,
}

impl Program {
    /// Starts the program to serve the login window, with the platform `windows`.
    ///
    /// The platform has to give the main screen without anyone picking it, and nothing else: a
    /// platform that also shows the windows of other devices, or unlocks screens, is refused,
    /// since either would put a question to a person, and nobody is signed in to be asked.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Platform`] if the platform is not as described, [`ErrorKind::Busy`] if
    /// another program holds the folder, and [`ErrorKind::Other`] if Baton could not start.
    pub async fn start(state: &State, windows: Windows) -> Result<Self> {
        let program = Self::start_without_platform(state, windows).await?;
        match check_platform(&program.app) {
            Ok(()) => Ok(program),
            Err(error) => {
                program.stop().await;
                Err(error)
            }
        }
    }

    /// Starts the program for what needs no screen: pairing a device, and nothing else. `windows`
    /// is whatever the caller has, which may be nothing.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Busy`] if another program holds the folder, and [`ErrorKind::Other`] if Baton
    /// could not start.
    pub async fn start_without_platform(state: &State, windows: Windows) -> Result<Self> {
        name_this_mac(state, None)?;
        let mut options = Options::new(state.path());
        // No folder for files and nothing that reads what the person works on, which is what
        // `Options::new` gives. And no links or text: there is no inbox to read them in.
        options.take_handoffs = false;
        // The screen is given as it is, which is what the record decides: the login window is not
        // covered and its keyboard is not held, since nothing private is on it and a hold there
        // could lock the person out.
        options.privacy_mode = false;
        let app = App::start_with(options, windows)
            .await
            .map_err(Error::from)?;
        Ok(Self { app })
    }

    /// The running Baton, for pairing devices with it.
    #[must_use]
    pub fn app(&self) -> &App {
        &self.app
    }

    /// Serves until `stop` is ready, and then stops politely, telling the devices that are
    /// connected.
    ///
    /// # Errors
    ///
    /// None today. It returns a result so that a caller does not change when it can fail.
    #[allow(clippy::unnecessary_wraps)]
    pub async fn serve(self, stop: impl Future<Output = ()>) -> Result<()> {
        let logging = tokio::spawn(log_sessions(self.app.events()));
        stop.await;
        self.stop().await;
        // The log ends with the events, which end when Baton has stopped.
        let _ = logging.await;
        Ok(())
    }

    /// Stops Baton, which lets go of the folder.
    pub async fn stop(self) {
        self.app.stop().await;
    }
}

/// Serves the login window from the folder `state`, with the platform `windows`, until `stop` is
/// ready. This is what the program does when launchd starts it.
///
/// # Errors
///
/// As for [`Program::start`].
pub async fn serve(state: &State, windows: Windows, stop: impl Future<Output = ()>) -> Result<()> {
    let program = Program::start(state, windows).await?;
    tracing::info!("Baton is serving the login window");
    program.serve(stop).await
}

/// Checks that the platform is what this program needs, and nothing more.
fn check_platform(app: &App) -> Result<()> {
    let abilities = app.window_abilities();
    if !abilities.can_give_screen {
        return Err(Error::new(
            ErrorKind::Platform,
            "There is no platform to give the screen of this Mac with, so there is nothing to \
             show. The program has to be started by the one that has it.",
        ));
    }
    if abilities.can_show || abilities.can_unlock || abilities.can_be_unlocked {
        return Err(Error::new(
            ErrorKind::Platform,
            "The platform does more than this program allows: it shows the windows of other \
             devices or unlocks screens. Either would put a question to a person, and nobody is \
             signed in at the login window to answer it.",
        ));
    }
    Ok(())
}

/// Says in the log that a session began and that it ended, and nothing more: who it was with and
/// what was typed in it are not for the log of a program that runs as root.
async fn log_sessions(mut events: Events) {
    while let Some(event) = events.next().await {
        match event {
            Event::WindowStarted(_) => tracing::info!("a session began"),
            Event::WindowEnded { .. } => tracing::info!("a session ended"),
            _ => {}
        }
    }
}
