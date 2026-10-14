// SPDX-License-Identifier: Apache-2.0

//! This device's clipboard.

use arboard::Clipboard;

use crate::error::CliError;

fn open() -> Result<Clipboard, CliError> {
    Clipboard::new().map_err(|e| {
        CliError::new(format!(
            "Could not reach the clipboard: {e}. (On Linux this needs a desktop session.)"
        ))
    })
}

/// The text on the clipboard.
pub fn read_text() -> Result<String, CliError> {
    match open()?.get_text() {
        Ok(text) if text.is_empty() => Err(CliError::new("The clipboard is empty.")),
        Ok(text) => Ok(text),
        Err(arboard::Error::ContentNotAvailable) => {
            Err(CliError::new("There is no text on the clipboard."))
        }
        Err(e) => Err(CliError::new(format!("Could not read the clipboard: {e}"))),
    }
}

/// Puts `text` on the clipboard.
///
/// On Linux this does not return until something else replaces the text, so
/// `while_waiting` is called first, to tell the person why nothing seems to happen. It is
/// not called elsewhere.
pub fn write_text(text: &str, while_waiting: impl FnOnce()) -> Result<(), CliError> {
    let mut clipboard = open()?;
    set(&mut clipboard, text, while_waiting)
        .map_err(|e| CliError::new(format!("Could not write to the clipboard: {e}")))
}

#[cfg(target_os = "linux")]
fn set(
    clipboard: &mut Clipboard,
    text: &str,
    while_waiting: impl FnOnce(),
) -> Result<(), arboard::Error> {
    use arboard::SetExtLinux;
    // On Linux the program that set the clipboard serves it to whoever pastes, so the
    // text is gone when this program exits unless a clipboard manager took it over.
    // Waiting here keeps it available until something else replaces it.
    while_waiting();
    clipboard.set().wait().text(text.to_owned())
}

#[cfg(not(target_os = "linux"))]
fn set(
    clipboard: &mut Clipboard,
    text: &str,
    _while_waiting: impl FnOnce(),
) -> Result<(), arboard::Error> {
    clipboard.set_text(text.to_owned())
}
