// SPDX-License-Identifier: Apache-2.0

//! Pairing a phone with the program, in Terminal, at the Mac.
//!
//! The login window has no screen of the program's own to ask the person on, so the person is in
//! Terminal. The program offers a ticket, the person gives it to the phone, and when the phone
//! asks to pair, the person compares the code that the program shows with the one on the phone
//! and answers here. Nothing else pairs a device: no offer is open until this runs, and nothing
//! that a device sends opens one.

use std::{
    future::Future,
    time::{SystemTime, UNIX_EPOCH},
};

use baton_app::{App, Device, Event, Offer, OfferOutcome, PairingRequest, Platform, StartAccess};
use baton_proto::device::sanitize_display_name;

use crate::{
    Error, ErrorKind, Result,
    person::{Person, is_yes},
};

/// How many characters of a device's id are enough to name it to `forget`.
const ID_PREFIX_CHARS: usize = 12;

/// Offers a ticket to the person and waits for a phone to pair with it.
///
/// The person gives the ticket to the phone, and answers here when the phone asks. A phone that
/// the person says yes to is paired, and allowed to start the main screen of this Mac with
/// control, which is all that it may do here ([`StartAccess::Control`]). A phone that is paired
/// is returned. `stop` ends the wait, and the offer with it.
///
/// # Errors
///
/// [`ErrorKind::Pairing`] if the person said no, the ticket ran out, the phone gave up or `stop`
/// came first, and [`ErrorKind::Other`] if the pairing could not be kept.
pub async fn pair(
    app: &App,
    person: &impl Person,
    stop: impl Future<Output = ()>,
) -> Result<Device> {
    tokio::pin!(stop);
    // Before the offer is made, so that nothing that happens to it is missed.
    let mut events = app.events();
    let offer = app.offer_pairing().await.map_err(Error::from)?;
    tell_the_ticket(person, &offer);

    loop {
        let event = tokio::select! {
            () = &mut stop => {
                app.cancel_offer();
                return Err(Error::new(ErrorKind::Pairing, "Stopped, and nothing was paired."));
            }
            event = events.next() => event,
        };
        match event {
            None => {
                return Err(Error::new(
                    ErrorKind::Pairing,
                    "Baton stopped before a device paired.",
                ));
            }
            Some(Event::PairingRequested(request)) => {
                show_request(person, &request);
                // The question is put with the codes on the screen, and the answer is waited for
                // here and nowhere else: whatever was typed before is not an answer to it.
                let answer = tokio::select! {
                    () = &mut stop => {
                        app.cancel_offer();
                        return Err(Error::new(
                            ErrorKind::Pairing,
                            "Stopped, and nothing was paired.",
                        ));
                    }
                    answer = person.ask() => answer,
                };
                let yes = is_yes(answer.as_deref());
                if !yes {
                    person.say("Declined.");
                }
                if !app.answer_pairing(request.id, yes) {
                    person.say("The request had ended before you answered.");
                }
            }
            Some(Event::OfferEnded(outcome)) => return finish(app, person, outcome),
            Some(_) => {}
        }
    }
}

/// Gives the person the ticket, on the screen and, where there is one, on the clipboard.
fn tell_the_ticket(person: &impl Person, offer: &Offer) {
    let minutes = minutes_left(offer.expires_at);
    person.say(
        "To pair a phone with the login window of this Mac, open Baton on the phone, choose \
         Add a device, then Enter code, and give it this ticket:",
    );
    person.say("");
    person.say(&format!("    {}", offer.ticket));
    person.say("");
    if person.copy(&offer.ticket) {
        person.say("The ticket is on the clipboard of this Mac as well.");
    }
    person.say(&format!(
        "The ticket works once and for about {minutes} {}. Whoever holds it can ask to pair, and \
         you still get to say no, but give it to no one else.",
        if minutes == 1 { "minute" } else { "minutes" }
    ));
    person.say("Waiting for the phone...");
}

/// Shows a device that asks to pair, and what the person has to compare.
fn show_request(person: &impl Person, request: &PairingRequest) {
    person.say("");
    person.say("A device wants to pair with the login window of this Mac:");
    person.say("");
    person.say(&format!(
        "    Name:         {}",
        sanitize_display_name(&request.name)
    ));
    person.say(&format!(
        "    System:       {}",
        platform_name(request.platform)
    ));
    person.say(&format!("    Fingerprint:  {}", request.fingerprint));
    person.say("");
    person.say("If it is yours, it is showing this code:");
    person.say("");
    person.say(&format!("    {}", request.code));
    person.say("");
    person.say("Type yes to pair only if the codes are the same. Anything else says no.");
}

/// What happened to the offer, as the person is told, and what comes of a pairing.
fn finish(app: &App, person: &impl Person, outcome: OfferOutcome) -> Result<Device> {
    match outcome {
        OfferOutcome::Paired(device) => {
            // What a device may do here is decided now, by the person who just said yes: it may
            // ask for the main screen and use it. A device that was paired and may do nothing is
            // not left behind if that cannot be kept.
            if let Err(error) = app.set_start_access(&device.id, StartAccess::Control) {
                let _ = app.unpair(&device.id);
                return Err(Error::new(
                    ErrorKind::Other,
                    format!(
                        "The device paired, but this program could not let it open the login \
                         window ({error}), so the pairing was taken back. Try again."
                    ),
                ));
            }
            let id_start: String = device.id.chars().take(ID_PREFIX_CHARS).collect();
            person.say("");
            person.say(&format!(
                "Paired with {} ({}).",
                sanitize_display_name(&device.name),
                device.fingerprint
            ));
            person.say(&format!(
                "When this Mac shows its login window, that device can look at it and use it. To \
                 end that, run `baton-loginwindow forget {id_start}`."
            ));
            Ok(device)
        }
        OfferOutcome::Expired => Err(pairing_error(
            "The ticket ran out before a device used it. Run the command again for a new one.",
        )),
        OfferOutcome::Cancelled => Err(pairing_error("The pairing was withdrawn.")),
        OfferOutcome::Declined => Err(pairing_error("You said no, so nothing was paired.")),
        OfferOutcome::Abandoned => Err(pairing_error(
            "The device gave up before you answered, so nothing was paired.",
        )),
        OfferOutcome::Failed(why) => Err(pairing_error(&format!("The pairing failed: {why}"))),
    }
}

fn pairing_error(message: &str) -> Error {
    Error::new(ErrorKind::Pairing, message)
}

/// How many whole minutes are left until `expires_at` (seconds since the Unix epoch), at least
/// one while there is any left.
fn minutes_left(expires_at: u64) -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    expires_at.saturating_sub(now).div_ceil(60).max(1)
}

fn platform_name(platform: Platform) -> &'static str {
    match platform {
        Platform::MacOs => "macOS",
        Platform::Windows => "Windows",
        Platform::Linux => "Linux",
        Platform::Ios => "iOS",
        Platform::IpadOs => "iPadOS",
        Platform::Android => "Android",
        Platform::Other => "an unknown system",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_time_left_is_whole_minutes_and_never_less_than_one() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // Away from a minute's edge, so that the second that may pass during the test does not
        // change the answer.
        assert_eq!(minutes_left(now + 300), 5);
        assert_eq!(minutes_left(now + 90), 2);
        assert_eq!(minutes_left(now + 5), 1);
        assert_eq!(minutes_left(now), 1);
        assert_eq!(
            minutes_left(0),
            1,
            "an offer that ended is not a negative time"
        );
    }

    /// A person who is told things, and keeps them.
    #[derive(Default)]
    struct Told(std::sync::Mutex<Vec<String>>);

    impl Person for Told {
        fn say(&self, text: &str) {
            self.0.lock().unwrap().push(text.to_owned());
        }

        fn copy(&self, _text: &str) -> bool {
            false
        }

        fn ask(&self) -> impl Future<Output = Option<String>> + Send {
            std::future::ready(None)
        }
    }

    #[test]
    fn what_a_device_calls_itself_cannot_change_the_screen_of_the_terminal() {
        let request = PairingRequest {
            id: 1,
            device_id: "ab".repeat(32),
            name: "Phone\u{1b}[2J\u{1b}[H\r\n    Name:         The Mac".to_owned(),
            platform: Platform::Ios,
            fingerprint: "abcd-efgh-ijkl-mnop-qrst".to_owned(),
            code: "514 643".to_owned(),
        };
        let told = Told::default();

        show_request(&told, &request);

        let lines = told.0.lock().unwrap().clone();
        assert!(
            lines.iter().all(|line| !line.chars().any(char::is_control)),
            "{lines:?}"
        );
        // The name is on the line that is for it, and cannot start a line of its own, such as one
        // that says whose name it is.
        let named = lines
            .iter()
            .filter(|line| line.trim_start().starts_with("Name:"))
            .count();
        assert_eq!(named, 1, "{lines:?}");
    }
}
