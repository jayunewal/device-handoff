// SPDX-License-Identifier: Apache-2.0

//! Pairing a phone with the program, in Terminal, at the Mac.
//!
//! The login window has no screen of the program's own to ask the person on, so the person is in
//! Terminal. The program offers a ticket, the person gives it to the phone, and when the phone
//! asks to pair, the person types here the six digits that the phone shows. The program pairs the
//! device only if they are the six digits that it worked out itself, and it does not show them,
//! so the person has to read them off the phone.
//!
//! Nothing on the screen may be taken for them either. A stranger who holds the ticket works out
//! the digits of its own pairing before it sends its name, and cleaning a name leaves digits in
//! it, so a name shown with the question could hold exactly what the person is to type. So the
//! question shows nothing that the device sent, but the system that it says it runs, which is one
//! of a few fixed words. The name and the fingerprint are shown once the pairing is made.
//!
//! That is also why nothing typed ahead pairs anything. A terminal keeps what is typed while the
//! program waits, so a `yes` typed before the phone asked would be waiting to answer the first
//! device that did. A line typed before cannot know the digits of a pairing that had not begun,
//! and a line that is not the digits says no. Nothing else pairs a device: no offer is open until
//! this runs, and nothing that a device sends opens one.

use std::{
    future::Future,
    time::{SystemTime, UNIX_EPOCH},
};

use baton_app::{App, Device, Event, Offer, OfferOutcome, PairingRequest, Platform, StartAccess};
use baton_proto::device::sanitize_display_name;

use crate::{Error, ErrorKind, Result, person::Person};

/// How many characters of a device's id are enough to name it to `forget`.
const ID_PREFIX_CHARS: usize = 12;

/// How many digits the code of a pairing has, which the person is asked to type.
const CODE_DIGITS: usize = 6;

/// Offers a ticket to the person and waits for a phone to pair with it.
///
/// The person gives the ticket to the phone, and when the phone asks to pair types here the six
/// digits that it shows. The program does not show them, nor anything that the device chose, and
/// a line that was typed before the phone asked is not them. A phone whose digits were typed is
/// paired, and allowed to start the main screen of this Mac with control, which is all that it
/// may do here ([`StartAccess::Control`]). A phone that is paired is returned. `stop` ends the
/// wait, and the offer with it.
///
/// # Errors
///
/// [`ErrorKind::Pairing`] if what was typed was not the digits (a blank line apart), the input
/// ended, the ticket ran out, the phone gave up or `stop` came first, and [`ErrorKind::Other`] if
/// the pairing could not be kept.
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
                // What is typed is waited for here and nowhere else, and only the digits of this
                // request pair it. A terminal also hands over what was typed before the request
                // came, but nothing had shown those digits yet, so such a line says no to the
                // device that asked first and pairs nothing.
                let typed = tokio::select! {
                    () = &mut stop => {
                        app.cancel_offer();
                        return Err(Error::new(
                            ErrorKind::Pairing,
                            "Stopped, and nothing was paired.",
                        ));
                    }
                    typed = next_line(person) => typed,
                };
                let right = match &typed {
                    Some(typed) if is_the_code(typed, &request.code) => true,
                    Some(_) => {
                        person.say("That is not what the phone shows.");
                        false
                    }
                    None => {
                        person.say("Nothing more can be typed.");
                        false
                    }
                };
                if !app.answer_pairing(request.id, right) {
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
        "The ticket works once and for about {minutes} {}. Whoever holds it can ask to pair, but \
         nothing is paired unless you type the six digits that the phone shows. Give it to no one \
         else.",
        if minutes == 1 { "minute" } else { "minutes" }
    ));
    person.say("Waiting for the phone...");
}

/// The next line that the person types that has something in it, or `None` if nothing more can be
/// typed. A Return alone is not an answer, so a person who presses it while they wait does not
/// say no by it.
async fn next_line(person: &impl Person) -> Option<String> {
    loop {
        let line = person.ask().await?;
        if !line.trim().is_empty() {
            return Some(line);
        }
    }
}

/// Whether `typed` is the code of the pairing: the six digits of `code`, which the phone shows
/// as two groups of three, with or without the space between them and with nothing else.
///
/// A code that is not six digits is not one that a person can be asked to type, so nothing
/// matches it.
fn is_the_code(typed: &str, code: &str) -> bool {
    let code: String = code.chars().filter(|c| *c != ' ').collect();
    let typed: String = typed.split_whitespace().collect();
    code.len() == CODE_DIGITS && code.chars().all(|c| c.is_ascii_digit()) && typed == code
}

/// Asks the person for the digits that the phone shows, when a device asks to pair.
///
/// The digits are not shown, so the person has to read them off the phone, and neither is
/// anything else that the device sent but the system it says it runs, which is one of a few fixed
/// words. Above all not its name: a stranger who holds the ticket works out the digits of its own
/// pairing before it sends its name, and a cleaned name keeps digits, so the name could be the
/// digits, in the form that a phone shows them, a few lines above where the person is told to type
/// them. The fingerprint, the id and the code are left out for the same reason, and nothing here
/// depends on them.
fn show_request(person: &impl Person, request: &PairingRequest) {
    person.say("");
    person.say(&format!(
        "A device wants to pair with the login window of this Mac. It says that it runs {}.",
        platform_name(request.platform)
    ));
    person.say("");
    person.say("If it is yours, it shows six digits. Type them here, and press Return.");
    person.say("Any other line says no, and nothing is paired. If it shows none, press Control-C.");
}

/// What happened to the offer, as the person is told, and what comes of a pairing.
fn finish(app: &App, person: &impl Person, outcome: OfferOutcome) -> Result<Device> {
    match outcome {
        OfferOutcome::Paired(device) => {
            // What a device may do here is decided now, by the person who just typed its digits:
            // it may ask for the main screen and use it. A device that was paired and may do
            // nothing is not left behind if that cannot be kept.
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
        OfferOutcome::Declined => Err(pairing_error(
            "The six digits that the phone shows were not typed, so nothing was paired. Run the \
             command again for a new ticket.",
        )),
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

    /// A person who is told things, and keeps them, and types the lines that they were given.
    #[derive(Default)]
    struct Told {
        said: std::sync::Mutex<Vec<String>>,
        typing: std::sync::Mutex<std::collections::VecDeque<String>>,
    }

    impl Told {
        /// A person who types these lines, one for each time that they are asked.
        fn typing(lines: &[&str]) -> Self {
            Self {
                said: std::sync::Mutex::default(),
                typing: std::sync::Mutex::new(
                    lines.iter().map(|line| (*line).to_owned()).collect(),
                ),
            }
        }

        fn said(&self) -> Vec<String> {
            self.said.lock().unwrap().clone()
        }
    }

    impl Person for Told {
        fn say(&self, text: &str) {
            self.said.lock().unwrap().push(text.to_owned());
        }

        fn copy(&self, _text: &str) -> bool {
            false
        }

        fn ask(&self) -> impl Future<Output = Option<String>> + Send {
            std::future::ready(self.typing.lock().unwrap().pop_front())
        }
    }

    fn a_request(code: &str) -> PairingRequest {
        PairingRequest {
            id: 1,
            device_id: "ab".repeat(32),
            name: "Pocket Phone".to_owned(),
            platform: Platform::Ios,
            fingerprint: "abcd-efgh-ijkl-mnop-qrst".to_owned(),
            code: code.to_owned(),
        }
    }

    /// What the person is told when `request` asks to pair.
    fn asked_for(request: &PairingRequest) -> Vec<String> {
        let told = Told::default();
        show_request(&told, request);
        told.said()
    }

    #[test]
    fn a_device_whose_name_is_its_own_code_does_not_put_the_code_on_the_screen() {
        // A stranger who holds the ticket works out the digits before it sends its name, and
        // cleaning a name keeps digits and a space, so the name can be the code as a phone shows
        // it. The core cleans it before the program sees it, which is done here too.
        for name in [
            "514 643",
            "514643",
            "Pocket Phone 514 643",
            "5 1 4 6 4 3",
            "My phone is 514 643",
        ] {
            let request = PairingRequest {
                name: sanitize_display_name(name),
                ..a_request("514 643")
            };
            assert!(
                request.name.chars().any(char::is_numeric),
                "{name:?}: cleaning took the digits out, so this tests nothing"
            );

            let text = asked_for(&request).join("\n");

            assert!(!text.contains("514"), "{name:?}: {text}");
            assert!(!text.contains("643"), "{name:?}: {text}");
            assert!(!text.contains(&request.name), "{name:?}: {text}");
        }
    }

    #[test]
    fn what_the_person_is_asked_does_not_depend_on_anything_that_the_device_sent_but_its_system() {
        let plain = asked_for(&a_request("514 643"));

        let names = [
            "514 643",
            "Pocket Phone 514 643",
            "Phone\u{1b}[2J\u{1b}[H\r\n    Name:         The Mac",
            "\u{ff15}\u{ff11}\u{ff14} \u{ff16}\u{ff14}\u{ff13}",
        ];
        for name in names {
            for (fingerprint, code) in [
                ("abcd-efgh-ijkl-mnop-qrst", "514 643"),
                ("ab23-4567-cdef-gh23-4567", "234 567"),
                ("a2b3-c4d5-e6f7-g2h3-i4j5", "999 000"),
            ] {
                let request = PairingRequest {
                    id: 2,
                    device_id: "cd".repeat(32),
                    name: name.to_owned(),
                    fingerprint: fingerprint.to_owned(),
                    code: code.to_owned(),
                    ..a_request("514 643")
                };

                assert_eq!(asked_for(&request), plain, "{name:?} {fingerprint} {code}");
            }
        }
        // What it says is what the program wrote: it tells the person what to type, and nothing in
        // it can move the cursor.
        assert!(
            plain.iter().any(|line| line.contains("Type them here")),
            "{plain:?}"
        );
        assert!(
            plain.iter().all(|line| !line.chars().any(char::is_control)),
            "{plain:?}"
        );
    }

    #[test]
    fn the_system_that_is_shown_has_no_digit_in_its_name() {
        for platform in [
            Platform::MacOs,
            Platform::Windows,
            Platform::Linux,
            Platform::Ios,
            Platform::IpadOs,
            Platform::Android,
            Platform::Other,
        ] {
            let name = platform_name(platform);
            assert!(
                !name.chars().any(char::is_numeric),
                "{name}: a digit in the one thing that is shown of the device could be mistaken \
                 for the code"
            );
        }
    }

    #[test]
    fn only_the_six_digits_of_the_pairing_are_the_code() {
        let code = "514 643";
        for typed in [
            "514 643",
            "514643",
            " 514 643 ",
            "514  643",
            "514\t643",
            "5 14 643",
            "514 643\r",
        ] {
            assert!(is_the_code(typed, code), "{typed:?}");
        }
        for typed in [
            "",
            " ",
            "yes",
            "y",
            "Yes",
            "no",
            "514",
            "514 64",
            "514 6433",
            "514 644",
            "415 643",
            "643 514",
            "514-643",
            "514 643 yes",
            "yes 514 643",
            "0514 643",
            "514 643x",
            "+514643",
            "\u{ff15}\u{ff11}\u{ff14} \u{ff16}\u{ff14}\u{ff13}",
            "514 643\u{0}",
        ] {
            assert!(!is_the_code(typed, code), "{typed:?}");
        }
    }

    #[test]
    fn a_code_that_is_not_six_digits_is_matched_by_nothing() {
        for code in [
            "",
            " ",
            "12345",
            "123 45",
            "1234567",
            "123 456 7",
            "abc def",
            "12a 456",
            "\u{ff11}\u{ff12}\u{ff13} \u{ff14}\u{ff15}\u{ff16}",
        ] {
            for typed in [code, "123456", "123 456", "12345", "", "abcdef", "12a456"] {
                assert!(!is_the_code(typed, code), "{code:?} typed as {typed:?}");
            }
        }
        // A code of six digits is matched, which is what the loop above would not show.
        assert!(is_the_code("123456", "123 456"));
        assert!(is_the_code("123456", "123456"));
    }

    #[tokio::test]
    async fn a_return_alone_is_not_an_answer_and_the_first_line_with_something_in_it_is() {
        let told = Told::typing(&["", "   ", "514 643", "yes"]);

        assert_eq!(next_line(&told).await.as_deref(), Some("514 643"));
        assert_eq!(next_line(&told).await.as_deref(), Some("yes"));
        assert_eq!(next_line(&told).await, None, "nothing more was typed");

        let only_returns = Told::typing(&["", ""]);
        assert_eq!(
            next_line(&only_returns).await,
            None,
            "returns are not something, and then the input ended"
        );
    }
}
