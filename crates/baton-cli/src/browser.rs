// SPDX-License-Identifier: Apache-2.0

//! Opening a received link.
//!
//! Another device chose the link, so it is opened only if it is a web link. Other
//! schemes can start programs or reach files (`file:`, `ms-msdt:`, `smb:`, a custom
//! scheme an app registered), and a person who asked for "open" on an item in a list
//! has not agreed to that. The receiver decides which schemes to open, as the
//! protocol says; this one opens `http` and `https`.

use url::Url;

use crate::{error::CliError, term};

/// The address to open, if `link` is a web link.
pub fn web_address(link: &str) -> Result<Url, CliError> {
    let url = Url::parse(link).map_err(|_| {
        CliError::new(format!(
            "\"{}\" is not a full web address.",
            term::line(link)
        ))
    })?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        other => Err(CliError::new(format!(
            "Only web links (http and https) are opened, and this one is a `{}` link. \
             `baton inbox copy` copies it, if you want to use it yourself.",
            term::line(other)
        ))),
    }
}

/// Opens `link` in the person's web browser.
pub fn open(link: &str) -> Result<Url, CliError> {
    let url = web_address(link)?;
    open::that_detached(url.as_str())
        .map_err(|e| CliError::new(format!("Could not open the browser: {e}")))?;
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_links_are_opened() {
        assert_eq!(
            web_address("https://example.com/a?b=c").unwrap().as_str(),
            "https://example.com/a?b=c"
        );
        assert!(web_address("http://example.com").is_ok());
        assert!(web_address("HTTPS://EXAMPLE.COM").is_ok());
    }

    #[test]
    fn other_schemes_are_not() {
        for link in [
            "file:///etc/passwd",
            "ms-msdt:/id PCWDiagnostic",
            "smb://server/share",
            "javascript:alert(1)",
            "mailto:a@example.com",
            "data:text/html,<script>1</script>",
            "obsidian://open?vault=x",
        ] {
            let error = web_address(link).unwrap_err();
            assert!(error.message().contains("Only web links"), "{link}");
        }
    }

    #[test]
    fn what_is_not_an_address_is_refused() {
        assert!(web_address("example.com").is_err());
        assert!(web_address("").is_err());
        assert!(web_address("not a link").is_err());
    }

    #[test]
    fn the_message_cannot_carry_escape_sequences() {
        let error = web_address("x\u{1b}[2J").unwrap_err();
        assert!(!error.message().contains('\u{1b}'));
    }
}
