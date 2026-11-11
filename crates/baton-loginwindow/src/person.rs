// SPDX-License-Identifier: Apache-2.0

//! The person at the Mac, as the program sees them: somewhere to say things, a clipboard and a
//! question to put.
//!
//! Nobody is signed in at the login window, so the person is in Terminal, running the program
//! with `sudo`. Tests put a script in their place.

use std::future::Future;

use tokio::io::{AsyncBufReadExt, BufReader};

/// The person who runs the program in Terminal.
pub trait Person {
    /// Tells the person something. The text ends with a new line.
    fn say(&self, text: &str);

    /// Puts `text` on the clipboard, for the person to paste on another device. Returns whether it
    /// worked. A person for whom it does not is told the text on screen as well, so nothing
    /// depends on it.
    fn copy(&self, text: &str) -> bool;

    /// Waits for the person to answer the question that was just put, with a line. `None` means
    /// that nothing more can be typed: the input has ended.
    fn ask(&self) -> impl Future<Output = Option<String>> + Send;
}

/// Whether `answer` is a yes: `yes` or `y`, in any case, with nothing else. Anything else, and no
/// answer at all, is a no.
pub(crate) fn is_yes(answer: Option<&str>) -> bool {
    answer.is_some_and(|answer| {
        let answer = answer.trim();
        answer.eq_ignore_ascii_case("yes") || answer.eq_ignore_ascii_case("y")
    })
}

/// The person at a terminal: what is said goes to standard output, and the answer is a line from
/// standard input.
#[derive(Debug, Clone, Copy, Default)]
pub struct Terminal;

impl Person for Terminal {
    fn say(&self, text: &str) {
        println!("{text}");
    }

    fn copy(&self, text: &str) -> bool {
        copy_to_clipboard(text)
    }

    async fn ask(&self) -> Option<String> {
        let mut line = String::new();
        match BufReader::new(tokio::io::stdin())
            .read_line(&mut line)
            .await
        {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line.trim().to_owned()),
        }
    }
}

/// Puts `text` on the clipboard of the person who is signed in at this Mac. Under `sudo`, the
/// program is in that person's session, so the clipboard is theirs.
#[cfg(target_os = "macos")]
fn copy_to_clipboard(text: &str) -> bool {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };

    // The full path: a program that runs as root does not look for another by name.
    let Ok(mut child) = Command::new("/usr/bin/pbcopy")
        .stdin(Stdio::piped())
        .spawn()
    else {
        return false;
    };
    let written = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
    // The write end is closed by now, which is what tells `pbcopy` that the text is complete.
    child.wait().is_ok_and(|status| status.success()) && written
}

/// There is no clipboard to reach from here.
#[cfg(not(target_os = "macos"))]
fn copy_to_clipboard(_text: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_yes_is_yes() {
        for yes in ["yes", "YES", "Yes", " yes ", "y", "Y", "yes\n"] {
            assert!(is_yes(Some(yes)), "{yes:?}");
        }
        for no in [
            "",
            " ",
            "no",
            "n",
            "yess",
            "ye",
            "y es",
            "yes please",
            "ok",
            "true",
            "1",
            "yes yes",
        ] {
            assert!(!is_yes(Some(no)), "{no:?}");
        }
        assert!(!is_yes(None), "no answer is a no");
    }
}
