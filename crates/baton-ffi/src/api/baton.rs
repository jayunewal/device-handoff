// SPDX-License-Identifier: Apache-2.0

//! The running Baton and the calls on it.
//!
//! Every function here is exposed to Dart by `flutter_rust_bridge_codegen`. The ones that
//! return a `Result` throw a `BatonError` in Dart. `&self` methods are instance methods of
//! the Dart class `Baton`; the others are static.

// The Dart side hands over owned values, and the generated glue cannot pass references.
#![allow(clippy::needless_pass_by_value)]

use std::path::{Path, PathBuf};

use baton_app::{App, Options};
use flutter_rust_bridge::{frb, spawn};

use super::{
    error::BatonError,
    model::{
        Device, Event, FileProgress, FilesSent, JoinProgress, Me, Network, Offer, Reach, Received,
        Receiving, ScreenStarted, StartAccess, WindowAbilities, WindowInput, WindowShared,
    },
};
use crate::convert::signed;
use crate::frb_generated::StreamSink;

/// One running Baton on this device.
///
/// Start it once, keep it for as long as the app runs, and `stop` it when the app is done.
#[frb(opaque)]
#[derive(Debug)]
pub struct Baton {
    app: App,
}

impl Baton {
    /// Opens the device's folder, starts the network and begins keeping what arrives.
    ///
    /// `data_dir` is a folder that belongs to the app alone. `lan_discovery` switches finding
    /// other devices on the local network on or off for this run, and `port_mapping`
    /// switches asking the router to open a port, whatever the settings file says; `None`
    /// leaves the settings file in charge. An iPhone or iPad passes `false` for both,
    /// because iOS only lets an app use the multicast messages that they start with when
    /// Apple has approved it for that.
    ///
    /// `receive_dir` is the folder that files sent by other devices are put in, which has to
    /// exist and be one the app may write to. `None` leaves this device taking no files. If
    /// Baton cannot use the folder it still starts, and `receiving` says why it takes no
    /// files.
    pub async fn start(
        data_dir: String,
        lan_discovery: Option<bool>,
        port_mapping: Option<bool>,
        receive_dir: Option<String>,
    ) -> Result<Baton, BatonError> {
        let mut options = Options::new(data_dir);
        options.lan_discovery = lan_discovery;
        options.port_mapping = port_mapping;
        options.receive_dir = receive_dir.map(PathBuf::from);
        // The device shows the windows that other devices share if the platform code of the app
        // registered a viewer for them before this call, and takes none otherwise.
        let windows = baton_stream_abi::registered_windows();
        let app = App::start_with(options, windows).await?;
        Ok(Self { app })
    }

    /// Stops the network, telling the devices that are connected, and lets go of the
    /// folder so that another `Baton` can start in it. Every later call throws
    /// `ErrorKind.stopped`, and the stream from `events` ends.
    pub async fn stop(&self) {
        self.app.stop().await;
    }

    /// This device.
    #[frb(sync)]
    pub fn me(&self) -> Me {
        self.app.me().into()
    }

    /// Where this device can be reached right now.
    pub fn network(&self) -> Network {
        self.app.network().into()
    }

    /// The devices paired with this one.
    pub fn devices(&self) -> Vec<Device> {
        self.app.devices().into_iter().map(Device::from).collect()
    }

    /// Forgets a paired device. It can no longer reach this one until it is paired again.
    pub fn unpair(&self, id: String) -> Result<(), BatonError> {
        Ok(self.app.unpair(&id)?)
    }

    /// Events from now on, as a stream that ends when the app stops. The call returns once
    /// the subscription is made, so `await` it before reading `devices` or `inbox`: nothing
    /// that happens after that is lost, even if the stream is not being listened to yet.
    ///
    /// The stream does not keep this `Baton` alive. One that is let go of without `stop` ends
    /// it and lets go of the folder, as the app layer does when it is dropped.
    #[frb(stream_dart_await)]
    pub async fn events(&self, sink: StreamSink<Event>) {
        let mut events = self.app.events();
        // The loop runs on its own, because a call that lasted as long as the stream would
        // hold on to this `Baton`, and only the end of the app or an event after Dart had
        // gone would let it go.
        spawn(async move {
            while let Some(event) = events.next().await {
                if sink.add(event.into()).is_err() {
                    // Dart stopped listening.
                    break;
                }
            }
        });
    }

    /// Opens an invitation to pair and returns it. Show `Offer.ticket` as a QR code and as
    /// text, and wait for `Event.pairingRequested` and then `Event.offerEnded`. An offer
    /// that is already showing is withdrawn.
    pub async fn offer_pairing(&self) -> Result<Offer, BatonError> {
        Ok(self.app.offer_pairing().await?.into())
    }

    /// Withdraws the offer that is showing, if there is one. `Event.offerEnded` follows.
    #[frb(sync)]
    pub fn cancel_offer(&self) {
        self.app.cancel_offer();
    }

    /// Answers a `PairingRequest`. Returns `false` if it is not waiting any more, because it
    /// was answered already or the offer ended.
    #[frb(sync)]
    pub fn answer_pairing(&self, request: i64, accept: bool) -> bool {
        u64::try_from(request).is_ok_and(|request| self.app.answer_pairing(request, accept))
    }

    /// Pairs with the device that made `ticket`: text as `Offer.ticket` shows it, or as a
    /// camera read it from the other device's QR code. Anything around the ticket, such as
    /// the rest of a pasted command, is ignored.
    ///
    /// The stream first gives `JoinProgress.code` once the other device has been reached,
    /// then `JoinProgress.paired` or `JoinProgress.failed` when the person there has
    /// answered, and ends.
    pub async fn join_pairing(&self, ticket: String, progress: StreamSink<JoinProgress>) {
        let code_sink = progress.clone();
        let result = self
            .app
            .join_pairing(&ticket, move |code| {
                // Nobody listening is fine.
                let _ = code_sink.add(JoinProgress::Code { code });
            })
            .await;
        let last = match result {
            Ok(device) => JoinProgress::Paired {
                device: device.into(),
            },
            Err(error) => JoinProgress::Failed {
                error: error.into(),
            },
        };
        let _ = progress.add(last);
    }

    /// Gives up on a `join_pairing` that is still waiting.
    #[frb(sync)]
    pub fn cancel_join(&self) {
        self.app.cancel_join();
    }

    /// Checks that a paired device answers, and how its traffic gets here.
    pub async fn ping(&self, id: String) -> Result<Reach, BatonError> {
        Ok(self.app.ping(&id).await?.into())
    }

    /// What this device does with windows: whether it shows the windows of other devices,
    /// shares its own, and lets the person grant control of the ones it shares. Offer only what
    /// is possible.
    #[frb(sync)]
    pub fn window_abilities(&self) -> WindowAbilities {
        self.app.window_abilities().into()
    }

    /// Answers a `WindowRequest`. Returns `false` if it is not waiting any more, because it
    /// was answered already or the other device gave up. A yes starts the window if this
    /// device can show it: `Event.windowStarted` says so, and `Event.windowRequestEnded` says
    /// why not.
    #[frb(sync)]
    pub fn answer_window(&self, request: i64, accept: bool) -> bool {
        u64::try_from(request).is_ok_and(|request| self.app.answer_window(request, accept))
    }

    /// Lets the person pick a window or a whole screen of this device, and offers it to the
    /// paired device `id`. The call returns when the person at the other device has answered,
    /// which can take as long as a person takes to decide. `control` lets the other device
    /// control the window or the screen, if `windowAbilities().canControl` says that this
    /// device can grant it. Throws `ErrorKind.notPaired`, `ErrorKind.unreachable`,
    /// `ErrorKind.declined` if the device does not show windows, and `ErrorKind.other` if this
    /// device does not share windows or a window or screen is shared or shown already.
    pub async fn share_window(
        &self,
        id: String,
        control: bool,
    ) -> Result<WindowShared, BatonError> {
        Ok(self.app.share_window(&id, control).await?.into())
    }

    /// Asks the paired device `id` to start sharing its screen with this one, and shows it when it
    /// does: `Event.windowStarted` says so, with a session that is shown here. Nobody at the other
    /// device is asked: the person there allowed this device beforehand (`setStartAccess`), or the
    /// answer is `ScreenStarted.notStarted`, with a sentence that says why. `control` is whether
    /// the person here wishes to control the screen; the other device grants no more than the
    /// settings for this one allow, so look at `WindowSession.control` to see what was granted.
    /// The call returns when the screen is shown or the answer is no. Throws `ErrorKind.notPaired`,
    /// `ErrorKind.unreachable`, `ErrorKind.declined` if the device does not know how to be asked,
    /// and `ErrorKind.other` if this device does not show screens or a window or screen is shared
    /// or shown already.
    pub async fn start_screen(
        &self,
        id: String,
        control: bool,
    ) -> Result<ScreenStarted, BatonError> {
        Ok(self.app.start_screen(&id, control).await?.into())
    }

    /// What the paired device `id` may do at this device without a person here being asked: ask
    /// it to start sharing its screen with that device. `StartAccess.off` for a device that was
    /// just paired. Throws `ErrorKind.notPaired`.
    #[frb(sync)]
    pub fn start_access(&self, id: String) -> Result<StartAccess, BatonError> {
        Ok(self.app.start_access(&id)?.into())
    }

    /// Lets the paired device `id` ask this device to start sharing its screen with it, to look at
    /// or to look at and control, or does not (`StartAccess.off`). Only the person at this device
    /// can say so: nothing a paired device sends changes it. The device can ask only if
    /// `windowAbilities().canGiveScreen` says that this device can give its screen; the setting is
    /// kept all the same. A screen that is shared is not stopped by turning this off. Throws
    /// `ErrorKind.notPaired`, and `ErrorKind.storage` if the setting could not be kept.
    pub fn set_start_access(&self, id: String, access: StartAccess) -> Result<(), BatonError> {
        Ok(self.app.set_start_access(&id, access.into())?)
    }

    /// What the person at this device said of the paired device `id` hearing the sound of this
    /// device's screen: `true` that it may, `false` that it may not, and `null` when they have not
    /// said, which means that it may in a session that lets it control the screen and may not in
    /// one that lets it only look, whoever started the session. Throws `ErrorKind.notPaired`.
    #[frb(sync)]
    pub fn hear_setting(&self, id: String) -> Result<Option<bool>, BatonError> {
        Ok(self.app.hear_setting(&id)?)
    }

    /// Says whether the paired device `id` may hear the sound of this device's screen, or takes
    /// what was said back with `null`. Only the person at this device can say so: nothing a paired
    /// device sends changes it. It counts the next time the device asks to hear, also in a session
    /// that runs, but a sound that plays is not stopped until then. Throws `ErrorKind.notPaired`,
    /// and `ErrorKind.storage` if the setting could not be kept.
    pub fn set_hear_setting(&self, id: String, setting: Option<bool>) -> Result<(), BatonError> {
        Ok(self.app.set_hear_setting(&id, setting)?)
    }

    /// Ends the window `session`, which is shown here or shared from here, and tells the other
    /// device. `Event.windowEnded` follows. Returns whether there was such a window.
    #[frb(sync)]
    pub fn end_window(&self, session: i64) -> bool {
        u64::try_from(session).is_ok_and(|session| self.app.end_window(session))
    }

    /// Ends the window `session`, which is shown here, and asks the device that shares it to
    /// leave its screen as it is when the session is over, and not lock it. It is for the person
    /// who was asked at the end of a session whose `WindowSession.leave` is set and answered that
    /// the screen stays as it is; every other end locks it, `Baton.endWindow` among them.
    /// `Event.windowEnded` follows. Returns whether the choice was made: this call ended the
    /// session, and the other device had offered it. The other device does not answer, so a true
    /// says that it was asked and not that the screen was left: if the connection goes first, the
    /// device locks. A false is a window that is not shown here, one that has ended already, or
    /// one whose device cannot leave its screen as it is; the last of these ends as
    /// `Baton.endWindow` ends it.
    #[frb(sync)]
    pub fn end_window_leaving(&self, session: i64) -> bool {
        u64::try_from(session).is_ok_and(|session| self.app.end_window_leaving(session))
    }

    /// Gives what the person did on the picture of the window `session`, which is shown here: a
    /// touch, a click, a scroll, a key. The event goes to the other device soon, in a batch with
    /// others, and is done there only if the person at that device let this one control the
    /// window (`WindowSession.control`). Returns whether the event was taken. It is not for a
    /// window that is not shown here or may not be controlled, a position while no picture has
    /// been shown, a position outside the picture while no button is down, the release of
    /// something that is not down, and more than the format allows. None of that ends the window:
    /// a screen drops the event and goes on. What the other device drops comes back as
    /// `Event.windowNotice`.
    #[frb(sync)]
    pub fn window_input(&self, session: i64, input: WindowInput) -> bool {
        u64::try_from(session).is_ok_and(|session| self.app.window_input(session, input.into()))
    }

    /// Lets go of every button and key that is down on the window `session`, for example when
    /// the person leaves the screen with a finger on it. A window that is not shown here does
    /// nothing.
    #[frb(sync)]
    pub fn release_window_input(&self, session: i64) {
        if let Ok(session) = u64::try_from(session) {
            self.app.release_window_input(session);
        }
    }

    /// Tells the device that shares the window `session`, which is shown here, what the people at
    /// that device may do while this one controls it: see its screen (`screen`) and use its
    /// keyboard and trackpad (`input`). It is for a window whose `WindowSession.private` is set.
    /// Both are off when the session starts, and each call says both, so the last one is how
    /// things stand: switch either on and off as often as you like, in one session. The device
    /// that shares the window locks itself and puts both back when the session ends, whatever
    /// the reason. Returns whether the message was taken, which is not so for a window that is not
    /// shown here, one that is not private, and a session whose other device has stopped reading,
    /// so that the person can be told that things are as they were.
    #[frb(sync)]
    pub fn window_lift(&self, session: i64, screen: bool, input: bool) -> bool {
        u64::try_from(session).is_ok_and(|session| self.app.window_lift(session, screen, input))
    }

    /// Asks the device that shares the screen `session`, which is shown here, to send its sound
    /// (`listen`), or to stop. The sound plays on this device, and the other device keeps its own
    /// speakers quiet for as long as it is sent. Sound is off when a session starts. It is for a
    /// session whose `WindowSession.sound` is set. Returns whether the request was made, which is
    /// not so for a window that is not shown here, for a window and not a whole screen, when this
    /// device plays no sound or the other sends none, and for a session that is over. How it came
    /// out comes as `Event.windowSound`. A request to hear that is made again asks the other
    /// device to try again, which a person does after it said that it could not, and gives this
    /// device a new run to try after it could not play.
    #[frb(sync)]
    pub fn hear_window(&self, session: i64, listen: bool) -> bool {
        u64::try_from(session).is_ok_and(|session| self.app.hear_window(session, listen))
    }

    /// Sends a web link to a paired device.
    pub async fn send_link(
        &self,
        id: String,
        url: String,
        title: Option<String>,
    ) -> Result<(), BatonError> {
        Ok(self.app.send_link(&id, &url, title.as_deref()).await?)
    }

    /// Sends text, such as what was on the clipboard, to a paired device.
    pub async fn send_text(&self, id: String, text: String) -> Result<(), BatonError> {
        Ok(self.app.send_text(&id, &text).await?)
    }

    /// Starts sending files and folders, given by their paths, to a paired device, and
    /// returns at once with the transfer that is going. The person at the other end is asked
    /// whether to take them. Look at `FileSend.progress` now and then to show how far the
    /// transfer has got, and wait for `FileSend.outcome` for the end.
    pub async fn send_files(&self, id: String, paths: Vec<String>) -> Result<FileSend, BatonError> {
        let paths = paths.into_iter().map(PathBuf::from).collect();
        Ok(FileSend {
            send: self.app.send_files(&id, paths).await?,
        })
    }

    /// Whether this device takes files from other devices, and if not, why. It is decided
    /// when Baton starts, from the `receive_dir` it was given.
    #[frb(sync)]
    pub fn receiving(&self) -> Receiving {
        self.app.receiving().into()
    }

    /// Answers a `TransferRequest`. Returns `false` if it is not waiting any more, because it
    /// was answered already, the other device gave up, or the person's two minutes ran out.
    #[frb(sync)]
    pub fn answer_transfer(&self, request: i64, accept: bool) -> bool {
        u64::try_from(request).is_ok_and(|request| self.app.answer_transfer(request, accept))
    }

    /// What other devices have sent, newest first.
    pub fn inbox(&self) -> Result<Vec<Received>, BatonError> {
        Ok(self.app.inbox()?.into_iter().map(Received::from).collect())
    }

    /// Removes one item from the inbox. Returns whether there was such an item.
    pub fn remove_received(&self, number: i64) -> Result<bool, BatonError> {
        match u64::try_from(number) {
            Ok(number) => Ok(self.app.remove_received(number)?),
            Err(_) => Ok(false),
        }
    }

    /// Removes everything from the inbox. Returns how many items it held.
    pub fn clear_inbox(&self) -> Result<i64, BatonError> {
        Ok(signed(self.app.clear_inbox()?))
    }
}

/// A transfer of files to another device that is going. Letting go of it before it has
/// ended gives the transfer up.
#[frb(opaque)]
#[derive(Debug)]
pub struct FileSend {
    send: baton_app::FileSend,
}

impl FileSend {
    /// How far the transfer has got. This is cheap, and the latest is always there.
    #[frb(sync)]
    pub fn progress(&self) -> FileProgress {
        self.send.progress().into()
    }

    /// Gives the transfer up, and tells the other device if it can. `outcome` then throws
    /// `ErrorKind.cancelled`. It does nothing once the transfer is over.
    #[frb(sync)]
    pub fn cancel(&self) {
        self.send.cancel();
    }

    /// Waits for the end of the transfer. It throws `ErrorKind.declined` if the other device
    /// did not take the files, `ErrorKind.unreachable` if Baton gave up reaching it,
    /// `ErrorKind.cancelled` if `cancel` was called, and the message says what happened.
    pub async fn outcome(&self) -> Result<FilesSent, BatonError> {
        Ok(self.send.outcome().await?.into())
    }
}

/// The address to open for a received link, if it is a web address, one that starts with
/// `http` or `https`; `null` otherwise. A received link is opened only if this returns an
/// address, and the address that comes back is the one to open, not the text that was sent.
/// The text was chosen by another device. `file:`, `tel:` and the schemes other apps register
/// can start programs, reach files or dial a number, and a different parser can read the same
/// text differently from the one that checked it. Offer to copy a link that comes back `null`.
#[frb(sync)]
pub fn web_address(url: String) -> Option<String> {
    baton_app::web_address(&url)
}

/// The name saved for this device in `data_dir`, if the person has chosen one. `null` means
/// none has been chosen yet; ask for one, and save it with `setDeviceName`, before starting.
pub fn device_name(data_dir: String) -> Result<Option<String>, BatonError> {
    Ok(baton_app::settings::device_name(Path::new(&data_dir))?)
}

/// Saves `name` as the name this device shows other devices. It takes effect the next time
/// Baton starts, and only for devices that pair after that. A name with nothing visible in
/// it is refused with `ErrorKind.badName`, and the message can be shown as it is.
pub fn set_device_name(data_dir: String, name: String) -> Result<(), BatonError> {
    Ok(baton_app::settings::set_device_name(
        Path::new(&data_dir),
        &name,
    )?)
}
