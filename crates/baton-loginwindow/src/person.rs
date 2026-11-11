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

    /// Waits for the person to type a line, and returns it. `None` means that nothing more can be
    /// typed: the input has ended.
    ///
    /// A terminal keeps what is typed while nothing asks, so the line may be one that the person
    /// typed before the question was put, and a line is not an answer to a question only because
    /// it came after it. A caller that has to be sure of an answer asks for something that cannot
    /// be typed before the question, such as the digits that only this pairing shows
    /// ([`pair()`](fn@crate::pair)).
    fn ask(&self) -> impl Future<Output = Option<String>> + Send;
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
