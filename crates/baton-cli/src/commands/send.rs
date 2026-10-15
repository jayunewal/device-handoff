// SPDX-License-Identifier: Apache-2.0

//! `baton send`.

use std::io::{IsTerminal, Read};

use baton_proto::message::{ClipboardText, DeclineReason, Item, Link, Outcome};
use url::Url;

use super::Mode;
use crate::{
    cli::SendCommand,
    clipboard,
    console::Console,
    context::Context,
    error::{CliError, unreachable},
    select, term,
};

/// Sends a link or some text to a paired device.
pub async fn run(
    ctx: &mut Context,
    _console: &Console,
    mode: Mode,
    command: SendCommand,
) -> Result<(), CliError> {
    // Say which device before anything slow happens, so a wrong name fails at once.
    let target = match &command {
        SendCommand::Link { to, .. }
        | SendCommand::Text { to, .. }
        | SendCommand::Clipboard { to } => to.to.clone(),
    };
    let devices = ctx.devices()?;
    let device = select::choose(&devices, target.as_deref())?;

    let (item, what) = match command {
        SendCommand::Link { url, title, .. } => (link(&url, title)?, "link"),
        SendCommand::Text { words, .. } => (text(mode, &words).await?, "text"),
        SendCommand::Clipboard { .. } => (
            Item::ClipboardText(ClipboardText {
                text: clipboard::read_text()?,
            }),
            "clipboard text",
        ),
    };

    let node = ctx.node().await?;
    let name = term::line(device.display_name());
    match node.send(&device.id, item).await {
        Ok(Outcome::Accepted) => {
            println!("Sent the {what} to {name}.");
            Ok(())
        }
        Ok(Outcome::Declined(reason)) => Err(CliError::declined(format!(
            "{name} did not take the {what}: {}.",
            why(reason)
        ))),
        Err(error) => Err(unreachable(error, &device)),
    }
}

/// A link, if `url` is a full address.
fn link(url: &str, title: Option<String>) -> Result<Item, CliError> {
    let url = url.trim();
    if Url::parse(url).is_err() {
        return Err(CliError::new(format!(
            "\"{}\" is not a full web address. Start it with https://",
            term::line(url)
        )));
    }
    Ok(Item::Link(Link {
        url: url.to_owned(),
        title: title.filter(|title| !title.trim().is_empty()),
    }))
}

/// The text to send: the words given, or else everything on standard input.
async fn text(mode: Mode, words: &[String]) -> Result<Item, CliError> {
    let text = if words.is_empty() {
        // At the prompt, standard input is where commands come from.
        if mode == Mode::InSession || std::io::stdin().is_terminal() {
            return Err(CliError::new(
                "There is no text to send. Put it after `send text`, in quotes if it has \
                 special characters, or pipe it in.",
            ));
        }
        let piped = tokio::task::spawn_blocking(|| {
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text).map(|_| text)
        })
        .await
        .map_err(|e| CliError::new(e.to_string()))??;
        // A pipe usually ends with the newline of the command that wrote it.
        let piped = piped.strip_suffix('\n').unwrap_or(&piped);
        piped.strip_suffix('\r').unwrap_or(piped).to_owned()
    } else {
        words.join(" ")
    };
    if text.is_empty() {
        return Err(CliError::new("There is no text to send."));
    }
    Ok(Item::ClipboardText(ClipboardText { text }))
}

/// What the other device said, as a person would put it.
fn why(reason: DeclineReason) -> &'static str {
    match reason {
        DeclineReason::Unsupported => {
            "it does not handle this kind of item, which an older version of Baton may not"
        }
        DeclineReason::TooLarge => "it is too large for that device to keep",
        DeclineReason::RateLimited => "it is receiving too fast from this device; wait a moment",
        DeclineReason::Blocked => "its settings for this device do not allow this kind of item",
        DeclineReason::Invalid => "it did not accept the content; for a link, check the address",
        DeclineReason::NoSpace => "it has no room for it",
        DeclineReason::Refused => "nobody there accepted it",
        DeclineReason::Other => "it gave a reason this version does not know",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_must_be_a_full_address() {
        assert!(link("https://example.com/a", None).is_ok());
        assert!(link("  https://example.com/a  ", None).is_ok());
        for bad in ["example.com", "/just/a/path", "", "not a url"] {
            let error = link(bad, None).unwrap_err();
            assert!(error.message().contains("full web address"), "{bad}");
        }
    }

    #[test]
    fn a_link_keeps_its_title_unless_it_is_blank() {
        let Ok(Item::Link(with)) = link("https://example.com/", Some("Example".to_owned())) else {
            panic!("not a link");
        };
        assert_eq!(with.title.as_deref(), Some("Example"));
        let Ok(Item::Link(blank)) = link("https://example.com/", Some("  ".to_owned())) else {
            panic!("not a link");
        };
        assert_eq!(blank.title, None);
    }

    #[tokio::test]
    async fn words_are_joined_with_single_spaces() {
        let words = ["hello".to_owned(), "there".to_owned()];
        let Item::ClipboardText(text) = text(Mode::OneShot, &words).await.unwrap() else {
            panic!("not text");
        };
        assert_eq!(text.text, "hello there");
    }

    #[tokio::test]
    async fn at_the_prompt_there_is_no_standard_input_to_fall_back_on() {
        let error = text(Mode::InSession, &[]).await.unwrap_err();
        assert!(error.message().contains("no text to send"));
    }

    #[test]
    fn every_reason_is_explained() {
        for reason in [
            DeclineReason::Unsupported,
            DeclineReason::TooLarge,
            DeclineReason::RateLimited,
            DeclineReason::Blocked,
            DeclineReason::Invalid,
            DeclineReason::Other,
        ] {
            assert_ne!(why(reason), "");
        }
    }
}
